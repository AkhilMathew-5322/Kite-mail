//! Background work: instant new-mail push (IMAP IDLE, one dedicated connection per account)
//! and task reminders. Both raise desktop notifications and emit events to the UI.
//! Threads sleep inside IDLE / sleep() so idle CPU use is effectively zero.

use crate::mail::Shared;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

/// Reads a boolean from the saved app settings (JSON blob).
pub fn setting_bool(st: &Shared, key: &str, default: bool) -> bool {
    st.db
        .get_setting("app")
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get(key).and_then(|x| x.as_bool()))
        .unwrap_or(default)
}

pub fn setting_str(st: &Shared, key: &str, default: &str) -> String {
    st.db
        .get_setting("app")
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get(key).and_then(|x| x.as_str()).map(String::from))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

pub fn start(app: AppHandle, st: Shared) {
    // Push supervisor: keeps exactly one IDLE thread per account.
    {
        let app = app.clone();
        let st = st.clone();
        std::thread::Builder::new().name("push-supervisor".into()).spawn(move || {
            type Worker = (Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);
            let mut running: HashMap<i64, Worker> = HashMap::new();
            loop {
                let ids: Vec<i64> = st.db.accounts().map(|a| a.into_iter().map(|a| a.id).collect()).unwrap_or_default();
                running.retain(|id, (stop, handle)| {
                    // A thread that ended on its own (e.g. a panic on a malformed server reply)
                    // is dropped here so it gets restarted below.
                    let alive = handle.as_ref().is_some_and(|h| !h.is_finished());
                    let keep = ids.contains(id) && alive && !stop.load(Ordering::Relaxed);
                    if !keep {
                        stop.store(true, Ordering::Relaxed);
                    }
                    keep
                });
                for id in ids {
                    if let std::collections::hash_map::Entry::Vacant(slot) = running.entry(id) {
                        let stop = Arc::new(AtomicBool::new(false));
                        let (app, st, flag) = (app.clone(), st.clone(), stop.clone());
                        let handle = std::thread::Builder::new()
                            .name(format!("idle-{id}"))
                            .spawn(move || idle_loop(app, st, id, flag))
                            .ok();
                        slot.insert((stop, handle));
                    }
                }
                std::thread::sleep(Duration::from_secs(30));
            }
        }).ok();
    }

    // Nextcloud: contacts/tasks sync every 15 minutes, scheduled backups checked hourly.
    {
        let app = app.clone();
        let st = st.clone();
        std::thread::Builder::new().name("nextcloud".into()).spawn(move || {
            std::thread::sleep(Duration::from_secs(20));
            let mut tick: u64 = 0;
            loop {
                if tick.is_multiple_of(4) {
                    for acct in st.db.accounts().unwrap_or_default() {
                        if crate::s3::backup_due(&crate::s3::load(&st.db), &acct.email) {
                            match backup_to_s3(&st, &acct, &|_| {}) {
                                Ok(dest) => { if setting_bool(&st, "notify_mail", true) { notify(&app, "Backup complete", &format!("{} → {dest}", acct.email)); } }
                                Err(e) => { let _ = app.emit("s3-error", format!("Backup of {} failed: {e}", acct.email)); }
                            }
                        }
                    }
                }
                let cfg = crate::nextcloud::load(&st.db);
                if cfg.enabled {
                    {
                        let _g = st.nc_lock.lock().unwrap_or_else(|p| p.into_inner());
                        match crate::nextcloud::sync(&st.db) {
                            Ok(rep) => { let _ = app.emit("nc-synced", &rep); }
                            Err(e) => { let _ = app.emit("nc-error", e); }
                        }
                    }
                    if tick.is_multiple_of(4) {
                        for acct in st.db.accounts().unwrap_or_default() {
                            if crate::nextcloud::backup_due(&crate::nextcloud::load(&st.db), &acct.email) {
                                match backup_to_nextcloud(&st, &acct, &|_| {}) {
                                    Ok(dest) => { if setting_bool(&st, "notify_mail", true) { notify(&app, "Backup complete", &format!("{} → Nextcloud/{dest}", acct.email)); } }
                                    Err(e) => { let _ = app.emit("nc-error", format!("Backup of {} failed: {e}", acct.email)); }
                                }
                            }
                        }
                    }
                }
                tick += 1;
                std::thread::sleep(Duration::from_secs(15 * 60));
            }
        }).ok();
    }

    // Task reminders.
    std::thread::Builder::new().name("reminders".into()).spawn(move || loop {
        if let Ok(due) = st.db.take_due_todos() {
            if !due.is_empty() {
                if setting_bool(&st, "notify_todos", true) {
                    for t in due.iter().take(5) {
                        let body = if t.notes.is_empty() { "Task is due now".to_string() } else { t.notes.chars().take(120).collect() };
                        notify(&app, &format!("⏰ {}", t.title), &body);
                    }
                }
                let _ = app.emit("todos-updated", json!({ "due": due.len() }));
            }
        }
        std::thread::sleep(Duration::from_secs(30));
    }).ok();
}

fn idle_loop(app: AppHandle, st: Shared, account_id: i64, stop: Arc<AtomicBool>) {
    let mut conn = None;
    let mut backoff = 15u64;
    while !stop.load(Ordering::Relaxed) {
        let Ok(acct) = st.db.account(account_id) else { break };
        let folder = st.db.folder_by_special(account_id, "inbox").ok().flatten().unwrap_or_else(|| "INBOX".into());

        match st.sync_collect_new(&acct, &folder, 100) {
            Ok(new) => {
                backoff = 15;
                let _ = app.emit("mail-updated", json!({ "account_id": account_id, "folder": folder }));
                // file attachments of new mail into the Files folder (account setting)
                let with_files: Vec<u32> = new.iter().filter(|m| m.has_attach).map(|m| m.uid).collect();
                if !with_files.is_empty() && st.db.account_prefs(account_id).collect_files {
                    match st.collect_from(&acct, &folder, &with_files) {
                        Ok(n) if n > 0 => { let _ = app.emit("files-updated", json!({ "account_id": account_id, "added": n })); }
                        Err(e) => eprintln!("collecting files failed: {e}"),
                        _ => {}
                    }
                }
                if !new.is_empty() && setting_bool(&st, "notify_mail", true) {
                    let _ = app.emit("new-mail", json!({ "account_id": account_id, "count": new.len() }));
                    let (title, body) = notification_text(&setting_str(&st, "notify_preview", "full"), &acct.email, &new);
                    notify(&app, &title, &body);
                }
            }
            Err(e) => {
                let _ = app.emit("account-error", json!({ "account_id": account_id, "error": e }));
                conn = None;
                nap(backoff, &stop);
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        }

        // Wait for the server to tell us something changed. The 5-minute cap means a
        // connection silently killed by sleep/Wi-Fi change is noticed within minutes.
        if let Err(_e) = st.idle_wait(&mut conn, &acct, &folder, IDLE_WAIT) {
            // Server without IDLE support or dropped connection: fall back to periodic checks.
            nap(backoff.max(60), &stop);
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }
}

/// Notification wording for `preview` = "full" (sender + subject), "sender" or "none".
pub fn notification_text(preview: &str, account: &str, new: &[crate::models::MsgSummary]) -> (String, String) {
    let who = |m: &crate::models::MsgSummary| if m.from_name.is_empty() { m.from_addr.clone() } else { m.from_name.clone() };
    match (preview, new.len()) {
        ("none", 1) => ("New mail".into(), account.to_string()),
        ("none", n) => (format!("{n} new messages"), account.to_string()),
        ("sender", 1) => (who(&new[0]), format!("New message · {account}")),
        ("sender", n) => {
            let mut names: Vec<String> = Vec::new();
            for m in new { let w = who(m); if !names.contains(&w) { names.push(w); } }
            (format!("{n} new messages · {account}"), names.into_iter().take(4).collect::<Vec<_>>().join(", "))
        }
        (_, 1) => (who(&new[0]), new[0].subject.clone()),
        (_, n) => (format!("{n} new messages · {account}"), new.iter().take(3).map(|m| format!("{}: {}", who(m), m.subject)).collect::<Vec<_>>().join("\n")),
    }
}

const IDLE_WAIT: Duration = Duration::from_secs(5 * 60);
const MAX_BACKOFF: u64 = 120;

/// Sleeps up to `secs`, waking early when the account is removed.
fn nap(secs: u64, stop: &AtomicBool) {
    for _ in 0..secs {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Full mailbox backup written to a temp file, uploaded to Nextcloud, then removed locally.
/// Backs up an account to the S3 bucket (records success or the error in the S3 settings).
pub fn backup_to_s3(st: &Shared, acct: &crate::models::Account, progress: &dyn Fn(crate::models::Progress)) -> Result<String, String> {
    let name = format!("kitemail-backup-{}-{}.zip", acct.email.replace('@', "_at_"), chrono::Local::now().format("%Y-%m-%d-%H%M"));
    let tmp = std::env::temp_dir().join(&name);
    let res = st.backup(acct, &tmp, progress).and_then(|_| crate::s3::upload_backup(&st.db, &tmp, &acct.email));
    let _ = std::fs::remove_file(&tmp);
    crate::s3::mark_backup(&st.db, &acct.email, res.as_ref().err().map(|e| e.as_str()))?;
    res
}

pub fn backup_to_nextcloud(st: &Shared, acct: &crate::models::Account, progress: &dyn Fn(crate::models::Progress)) -> Result<String, String> {
    let name = format!("kitemail-backup-{}-{}.zip", acct.email.replace('@', "_at_"), chrono::Local::now().format("%Y-%m-%d"));
    let tmp = std::env::temp_dir().join(&name);
    st.backup(acct, &tmp, progress)?;
    let res = crate::nextcloud::upload_backup(&st.db, &tmp, &acct.email);
    let _ = std::fs::remove_file(&tmp);
    let dest = res?;
    crate::nextcloud::mark_backup(&st.db, &acct.email)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::notification_text;
    use crate::models::MsgSummary;

    fn m(name: &str, subj: &str) -> MsgSummary {
        MsgSummary { account_id: 1, folder: "INBOX".into(), uid: 1, subject: subj.into(), from_name: name.into(), from_addr: "x@y".into(),
            ..Default::default() }
    }

    #[test]
    fn notification_privacy_levels() {
        let one = [m("Sara", "Salary review")];
        assert_eq!(notification_text("full", "me@x", &one), ("Sara".into(), "Salary review".into()));
        assert!(!notification_text("sender", "me@x", &one).1.contains("Salary"));
        let none = notification_text("none", "me@x", &one);
        assert!(!none.0.contains("Sara") && !none.1.contains("Salary"));
        let two = [m("Sara", "A"), m("Sara", "B")];
        assert_eq!(notification_text("sender", "me@x", &two).1, "Sara");
    }
}
