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
            let mut running: HashMap<i64, Arc<AtomicBool>> = HashMap::new();
            loop {
                let ids: Vec<i64> = st.db.accounts().map(|a| a.into_iter().map(|a| a.id).collect()).unwrap_or_default();
                running.retain(|id, stop| {
                    let keep = ids.contains(id) && !stop.load(Ordering::Relaxed);
                    if !keep {
                        stop.store(true, Ordering::Relaxed);
                    }
                    keep
                });
                for id in ids {
                    if !running.contains_key(&id) {
                        let stop = Arc::new(AtomicBool::new(false));
                        running.insert(id, stop.clone());
                        let (app, st) = (app.clone(), st.clone());
                        let _ = std::thread::Builder::new()
                            .name(format!("idle-{id}"))
                            .spawn(move || idle_loop(app, st, id, stop));
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
                let cfg = crate::nextcloud::load(&st.db);
                if cfg.enabled {
                    {
                        let _g = st.nc_lock.lock().unwrap_or_else(|p| p.into_inner());
                        match crate::nextcloud::sync(&st.db) {
                            Ok(rep) => { let _ = app.emit("nc-synced", &rep); }
                            Err(e) => { let _ = app.emit("nc-error", e); }
                        }
                    }
                    if tick % 4 == 0 {
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
                if !new.is_empty() && setting_bool(&st, "notify_mail", true) {
                    if new.len() == 1 {
                        let m = &new[0];
                        let from = if m.from_name.is_empty() { &m.from_addr } else { &m.from_name };
                        notify(&app, from, &m.subject);
                    } else {
                        let lines: Vec<String> = new.iter().take(3).map(|m| {
                            let from = if m.from_name.is_empty() { &m.from_addr } else { &m.from_name };
                            format!("{from}: {}", m.subject)
                        }).collect();
                        notify(&app, &format!("{} new messages · {}", new.len(), acct.email), &lines.join("\n"));
                    }
                }
            }
            Err(e) => {
                let _ = app.emit("account-error", json!({ "account_id": account_id, "error": e }));
                conn = None;
                std::thread::sleep(Duration::from_secs(backoff));
                backoff = (backoff * 2).min(600);
                continue;
            }
        }

        // Wait for the server to tell us something changed (or re-check every 20 minutes).
        if let Err(_e) = st.idle_wait(&mut conn, &acct, &folder, Duration::from_secs(20 * 60)) {
            // Server without IDLE support or dropped connection: fall back to periodic checks.
            std::thread::sleep(Duration::from_secs(backoff.max(120)));
            backoff = (backoff * 2).min(600);
        }
    }
}

/// Full mailbox backup written to a temp file, uploaded to Nextcloud, then removed locally.
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
