//! Core mail logic: authentication, IMAP sync, message actions, SMTP sending.
//! All functions here are blocking and are called from `spawn_blocking` in commands.rs.

use crate::db::Db;
use crate::mime;
use crate::models::*;
use crate::oauth;
use crate::secrets;
use imap::types::{Flag, NameAttribute};
use std::collections::{HashMap, HashSet};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Session = imap::Session<native_tls::TlsStream<TcpStream>>;

pub struct Conn {
    sess: Session,
    selected: Option<String>,
    has_move: bool,
}

impl Conn {
    pub fn logout(mut self) {
        let _ = self.sess.logout();
    }
}

pub struct AppState {
    pub db: Db,
    /// Serialises Nextcloud syncs (background timer vs. "Sync now").
    pub nc_lock: Mutex<()>,
    sessions: Mutex<HashMap<i64, Arc<Mutex<Option<Conn>>>>>,
    tokens: Mutex<HashMap<i64, (String, Instant)>>,
}

pub type Shared = Arc<AppState>;

pub enum Auth {
    Password(String),
    Bearer(String),
}

const HEADER_QUERY: &str = "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])";

impl AppState {
    pub fn new(db: Db) -> Self {
        AppState { db, nc_lock: Mutex::new(()), sessions: Mutex::new(HashMap::new()), tokens: Mutex::new(HashMap::new()) }
    }

    // ------------------------------------------------------------------ auth

    pub fn auth_for(&self, acct: &Account) -> Result<Auth, String> {
        if !acct.is_oauth() {
            return secrets::get(&self.db, &secrets::password_key(acct.id))
                .map(Auth::Password)
                .ok_or_else(|| "Password not found - remove and re-add the account".into());
        }
        if let Some((tok, exp)) = self.tokens.lock().unwrap().get(&acct.id) {
            if *exp > Instant::now() + Duration::from_secs(120) {
                return Ok(Auth::Bearer(tok.clone()));
            }
        }
        let refresh = secrets::get(&self.db, &secrets::refresh_key(acct.id))
            .ok_or("Not signed in - click Reconnect on this account")?;
        let p = oauth::provider(&acct.kind, &acct.tenant, &acct.client_id, &acct.client_secret)?;
        let t = oauth::refresh(&p, &refresh).map_err(|e| format!("{e}\nClick Reconnect on this account to sign in again."))?;
        self.store_tokens(acct.id, &t)?;
        Ok(Auth::Bearer(t.access_token))
    }

    pub fn store_tokens(&self, account_id: i64, t: &oauth::TokenResponse) -> Result<(), String> {
        if let Some(r) = &t.refresh_token {
            secrets::set(&self.db, &secrets::refresh_key(account_id), r)?;
        }
        let exp = Instant::now() + Duration::from_secs(t.expires_in.unwrap_or(3600));
        self.tokens.lock().unwrap().insert(account_id, (t.access_token.clone(), exp));
        Ok(())
    }

    pub fn forget_account(&self, account_id: i64) {
        self.tokens.lock().unwrap().remove(&account_id);
        if let Some(slot) = self.sessions.lock().unwrap().remove(&account_id) {
            if let Some(mut c) = slot.lock().unwrap().take() {
                let _ = c.sess.logout();
            }
        }
    }

    // ------------------------------------------------------------------ IMAP session handling

    /// Runs `f` with a live IMAP connection for the account. Reconnects once if the
    /// connection dropped (timeouts, server restarts, expired OAuth sessions).
    fn with_imap<T>(&self, acct: &Account, mut f: impl FnMut(&mut Conn) -> imap::Result<T>) -> Result<T, String> {
        let slot = self.sessions.lock().unwrap().entry(acct.id).or_default().clone();
        let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());
        for attempt in 0..2 {
            if guard.is_none() {
                let auth = self.auth_for(acct)?;
                *guard = Some(connect(acct, &auth)?);
            }
            let conn = guard.as_mut().unwrap();
            match f(conn) {
                Ok(v) => {
                    while conn.sess.unsolicited_responses.try_recv().is_ok() {}
                    return Ok(v);
                }
                Err(e) => {
                    let retryable = matches!(e, imap::Error::Io(_) | imap::Error::ConnectionLost | imap::Error::Bad(_) | imap::Error::Parse(_));
                    *guard = None;
                    if attempt == 0 && retryable {
                        continue;
                    }
                    return Err(format!("IMAP error: {e}"));
                }
            }
        }
        unreachable!()
    }

    // ------------------------------------------------------------------ folders

    pub fn refresh_folders(&self, acct: &Account) -> Result<Vec<Folder>, String> {
        let raw: Vec<(String, String, Vec<String>)> = self.with_imap(acct, |c| {
            let names = c.sess.list(Some(""), Some("*"))?;
            Ok(names
                .iter()
                .map(|n| {
                    let attrs = n
                        .attributes()
                        .iter()
                        .map(|a| match a {
                            NameAttribute::NoSelect => "\\Noselect".to_string(),
                            NameAttribute::NoInferiors => "\\Noinferiors".to_string(),
                            NameAttribute::Marked => "\\Marked".to_string(),
                            NameAttribute::Unmarked => "\\Unmarked".to_string(),
                            NameAttribute::Custom(s) => s.to_string(),
                        })
                        .collect();
                    (n.name().to_string(), n.delimiter().unwrap_or("/").to_string(), attrs)
                })
                .collect())
        })?;
        let folders = organize_folders(acct.id, raw);
        self.db.replace_folders(acct.id, &folders)?;
        self.db.folders(acct.id)
    }

    // ------------------------------------------------------------------ sync

    /// Syncs the newest `limit` messages of a folder: new headers, flag changes, deletions.
    pub fn sync_folder(&self, acct: &Account, folder: &str, limit: u32) -> Result<(), String> {
        let db = &self.db;
        let known_validity = db.uidvalidity(acct.id, folder)?;
        let cached = db.cached_uids(acct.id, folder)?;

        let (validity, all_uids, headers, flags) = self.with_imap(acct, |c| {
            let mb = c.sess.select(folder)?;
            c.selected = Some(folder.to_string());
            let validity = mb.uid_validity.unwrap_or(0);
            let cached_now: HashSet<u32> = if validity != known_validity { HashSet::new() } else { cached.clone() };

            let all: HashSet<u32> = if mb.exists == 0 { HashSet::new() } else { c.sess.uid_search("ALL")? };
            let mut sorted: Vec<u32> = all.iter().copied().collect();
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            sorted.truncate(limit as usize);
            // UIDs follow arrival order, not message date: imported/restored mail (PST imports,
            // migrations) can push recent messages outside the newest-UID window. Always include
            // everything dated in the last 14 days as well.
            if mb.exists > 0 {
                let since = (chrono::Utc::now() - chrono::Duration::days(14)).format("%d-%b-%Y").to_string();
                if let Ok(recent) = c.sess.uid_search(format!("SINCE {since}")) {
                    let mut recent: Vec<u32> = recent.into_iter().collect();
                    recent.sort_unstable_by(|a, b| b.cmp(a));
                    let have: HashSet<u32> = sorted.iter().copied().collect();
                    sorted.extend(recent.into_iter().take(500).filter(|u| !have.contains(u)));
                }
            }

            let new: Vec<u32> = sorted.iter().copied().filter(|u| !cached_now.contains(u)).collect();
            let existing: Vec<u32> = sorted.iter().copied().filter(|u| cached_now.contains(u)).collect();

            let mut headers = Vec::new();
            for chunk in new.chunks(100) {
                let fetches = c.sess.uid_fetch(uid_set(chunk), HEADER_QUERY)?;
                for f in fetches.iter() {
                    let Some(uid) = f.uid else { continue };
                    let h = mime::parse_headers(f.header().unwrap_or(b""));
                    let (seen, flagged, answered) = flag_bits(f.flags());
                    let date = if h.date > 0 { h.date } else { f.internal_date().map(|d| d.timestamp()).unwrap_or(0) };
                    headers.push(MsgSummary {
                        account_id: acct.id,
                        folder: folder.to_string(),
                        uid,
                        subject: h.subject,
                        from_name: h.from_name,
                        from_addr: h.from_addr,
                        to: h.to,
                        date,
                        size: f.size.unwrap_or(0),
                        seen,
                        flagged,
                        answered,
                        has_attach: h.has_attach,
                        preview: String::new(),
                    });
                }
            }
            let mut flags = Vec::new();
            for chunk in existing.chunks(500) {
                let fetches = c.sess.uid_fetch(uid_set(chunk), "(UID FLAGS)")?;
                for f in fetches.iter() {
                    if let Some(uid) = f.uid {
                        let (s, fl, an) = flag_bits(f.flags());
                        flags.push((uid, s, fl, an));
                    }
                }
            }
            Ok((validity, all, headers, flags))
        })?;

        if validity != known_validity {
            db.reset_folder(acct.id, folder, validity)?;
        } else {
            let gone: Vec<u32> = cached.iter().copied().filter(|u| !all_uids.contains(u)).collect();
            db.delete_messages(acct.id, folder, &gone)?;
        }
        db.upsert_summaries(&headers)?;
        db.update_flags(acct.id, folder, &flags)?;
        Ok(())
    }

    /// Downloads full bodies of recent messages so they can be read offline.
    pub fn prefetch_bodies(&self, acct: &Account, folder: &str, count: u32) -> Result<u32, String> {
        let uids = self.db.uids_without_body(acct.id, folder, 2_000_000, count)?;
        if uids.is_empty() {
            return Ok(0);
        }
        let bodies = self.with_imap(acct, |c| {
            ensure_selected(c, folder)?;
            let fetches = c.sess.uid_fetch(uid_set(&uids), "(UID BODY.PEEK[])")?;
            Ok(fetches.iter().filter_map(|f| Some((f.uid?, f.body()?.to_vec()))).collect::<Vec<_>>())
        })?;
        for (uid, raw) in &bodies {
            self.db.store_raw(acct.id, folder, *uid, raw, &mime::search_text(raw))?;
        }
        Ok(bodies.len() as u32)
    }

    // ------------------------------------------------------------------ messages

    pub fn raw_message(&self, acct: &Account, folder: &str, uid: u32) -> Result<Vec<u8>, String> {
        if let Some(raw) = self.db.raw(acct.id, folder, uid)? {
            return Ok(raw);
        }
        let raw = self.with_imap(acct, |c| {
            ensure_selected(c, folder)?;
            let fetches = c.sess.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")?;
            Ok(fetches.iter().find_map(|f| f.body().map(|b| b.to_vec())))
        })?
        .ok_or("Message no longer exists on the server")?;
        self.db.store_raw(acct.id, folder, uid, &raw, &mime::search_text(&raw))?;
        Ok(raw)
    }

    pub fn open_message(&self, acct: &Account, folder: &str, uid: u32) -> Result<MessageView, String> {
        let summary = self.db.summary(acct.id, folder, uid)?.ok_or("Message not in cache - refresh the folder")?;
        let raw = self.raw_message(acct, folder, uid)?;
        if !summary.seen {
            // Best effort: mark as read on the server (works offline too - local state updates).
            let _ = self.set_flag(acct, folder, &[uid], "seen", true);
        }
        let mut view = mime::build_view(&raw, summary)?;
        view.summary.seen = true;
        Ok(view)
    }

    pub fn set_flag(&self, acct: &Account, folder: &str, uids: &[u32], flag: &str, value: bool) -> Result<(), String> {
        let imap_flag = match flag {
            "seen" => "\\Seen",
            "flagged" => "\\Flagged",
            "answered" => "\\Answered",
            _ => return Err("Unknown flag".into()),
        };
        self.db.set_flag(acct.id, folder, uids, flag, value)?;
        let op = if value { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
        self.with_imap(acct, |c| {
            ensure_selected(c, folder)?;
            c.sess.uid_store(uid_set(uids), format!("{op} ({imap_flag})"))?;
            Ok(())
        })
    }

    /// Moves to Trash, or deletes permanently when already in Trash (or no Trash folder exists).
    pub fn delete(&self, acct: &Account, folder: &str, uids: &[u32]) -> Result<(), String> {
        let trash = self.db.folder_by_special(acct.id, "trash")?;
        let set = uid_set(uids);
        self.with_imap(acct, |c| {
            ensure_selected(c, folder)?;
            match &trash {
                Some(t) if t != folder => {
                    if c.has_move {
                        c.sess.uid_mv(&set, t)?;
                    } else {
                        c.sess.uid_copy(&set, t)?;
                        c.sess.uid_store(&set, "+FLAGS.SILENT (\\Deleted)")?;
                        c.sess.expunge()?;
                    }
                }
                _ => {
                    c.sess.uid_store(&set, "+FLAGS.SILENT (\\Deleted)")?;
                    c.sess.expunge()?;
                }
            }
            Ok(())
        })?;
        self.db.delete_messages(acct.id, folder, uids)
    }

    pub fn move_to(&self, acct: &Account, folder: &str, uids: &[u32], target: &str) -> Result<(), String> {
        if folder == target {
            return Ok(());
        }
        let set = uid_set(uids);
        self.with_imap(acct, |c| {
            ensure_selected(c, folder)?;
            if c.has_move {
                c.sess.uid_mv(&set, target)?;
            } else {
                c.sess.uid_copy(&set, target)?;
                c.sess.uid_store(&set, "+FLAGS.SILENT (\\Deleted)")?;
                c.sess.expunge()?;
            }
            Ok(())
        })?;
        self.db.delete_messages(acct.id, folder, uids)
    }

    // ------------------------------------------------------------------ sending

    pub fn send(&self, acct: &Account, draft: &Draft) -> Result<(), String> {
        let mut linked;
        let draft = if draft.link_large && !draft.attachments.is_empty() {
            linked = clone_draft(draft);
            crate::nextcloud::linkify_attachments(&self.db, &mut linked.body, &mut linked.attachments)?;
            &linked
        } else {
            draft
        };
        let mut extra = Vec::new();
        if let Some(fwd) = &draft.forward_from {
            let raw = self.raw_message(acct, &fwd.folder, fwd.uid)?;
            extra = mime::all_attachments(&raw);
        }
        let msg = crate::smtp::build(acct, draft, extra)?;
        let auth = self.auth_for(acct)?;
        crate::smtp::send(acct, &auth, &msg)?;

        // Gmail and Microsoft 365 save sent mail automatically; other servers need an APPEND.
        if acct.save_sent {
            if let Some(sent) = self.db.folder_by_special(acct.id, "sent")? {
                let bytes = msg.formatted();
                let _ = self.with_imap(acct, |c| c.sess.append_with_flags(&sent, &bytes, &[Flag::Seen]));
            }
        }
        if let Some(r) = &draft.reply_to_msg {
            let _ = self.set_flag(acct, &r.folder, &[r.uid], "answered", true);
        }
        if let Some(d) = &draft.draft {
            let _ = self.expunge_uids(acct, &d.folder, &[d.uid]);
        }
        if let Some(id) = draft.local_draft_id {
            self.db.delete_local_draft(id);
        }
        Ok(())
    }

    /// Permanently removes messages (used for replaced / sent drafts).
    fn expunge_uids(&self, acct: &Account, folder: &str, uids: &[u32]) -> Result<(), String> {
        let set = uid_set(uids);
        self.with_imap(acct, |c| {
            ensure_selected(c, folder)?;
            c.sess.uid_store(&set, "+FLAGS.SILENT (\\Deleted)")?;
            if c.sess.uid_expunge(&set).is_err() {
                c.sess.expunge()?;
            }
            Ok(())
        })?;
        self.db.delete_messages(acct.id, folder, uids)
    }

    // ------------------------------------------------------------------ drafts

    /// Saves a draft locally (always) and to the server's Drafts folder (when reachable).
    /// The previous server copy is replaced.
    pub fn save_draft(&self, acct: &Account, d: &Draft) -> Result<DraftSaved, String> {
        let message_id = d.message_id.clone().filter(|m| !m.is_empty()).unwrap_or_else(new_message_id);
        let json = serde_json::json!({
            "account_id": d.account_id, "to": d.to, "cc": d.cc, "bcc": d.bcc, "subject": d.subject, "body": d.body,
            "in_reply_to": d.in_reply_to, "references": d.references, "attachments": d.attachments, "message_id": message_id,
        }).to_string();
        let local_id = self.db.save_local_draft(d.local_draft_id, &json, true)?;

        let drafts_folder = match self.db.folder_by_special(acct.id, "drafts")? {
            Some(f) => f,
            None => return Ok(DraftSaved { local_draft_id: local_id, draft: None, message_id, server_error: Some("No Drafts folder on server".into()) }),
        };
        let server = (|| -> Result<Option<u32>, String> {
            let mut extra = Vec::new();
            if let Some(prev) = &d.draft {
                // keep attachments that were already part of the server draft
                if let Ok(raw) = self.raw_message(acct, &prev.folder, prev.uid) {
                    extra = mime::all_attachments(&raw);
                }
            }
            let msg = crate::smtp::build_draft(acct, &Draft { message_id: Some(message_id.clone()), ..clone_draft(d) }, extra)?;
            let bytes = msg.formatted();
            let mid = message_id.clone();
            let uid = self.with_imap(acct, |c| {
                c.sess.append_with_flags(&drafts_folder, &bytes, &[Flag::Seen, Flag::Draft])?;
                c.selected = None;
                c.sess.select(&drafts_folder)?;
                c.selected = Some(drafts_folder.clone());
                let found = c.sess.uid_search(format!("HEADER Message-ID \"{}\"", mid.trim_matches(|ch| ch == '<' || ch == '>')))?;
                Ok(found.into_iter().max())
            })?;
            if let Some(prev) = &d.draft {
                if Some(prev.uid) != uid || prev.folder != drafts_folder {
                    let _ = self.expunge_uids(acct, &prev.folder, &[prev.uid]);
                }
            }
            Ok(uid)
        })();
        match server {
            Ok(uid) => {
                self.db.save_local_draft(Some(local_id), &json, false)?;
                Ok(DraftSaved {
                    local_draft_id: local_id,
                    draft: uid.map(|u| MsgRefOut { folder: drafts_folder, uid: u }),
                    message_id,
                    server_error: None,
                })
            }
            Err(e) => Ok(DraftSaved { local_draft_id: local_id, draft: d.draft.as_ref().map(|r| MsgRefOut { folder: r.folder.clone(), uid: r.uid }), message_id, server_error: Some(e) }),
        }
    }

    pub fn discard_draft(&self, acct: &Account, draft: Option<&MsgRef>, local_id: Option<i64>) -> Result<(), String> {
        if let Some(id) = local_id {
            self.db.delete_local_draft(id);
        }
        if let Some(d) = draft {
            self.expunge_uids(acct, &d.folder, &[d.uid])?;
        }
        Ok(())
    }

    pub fn open_draft(&self, acct: &Account, folder: &str, uid: u32) -> Result<DraftView, String> {
        let raw = self.raw_message(acct, folder, uid)?;
        mime::draft_view(acct.id, &raw)
    }

    // ------------------------------------------------------------------ archive / folders

    /// Archive target: the server's Archive folder, Gmail's All Mail, or a newly created "Archive".
    pub fn archive_folder(&self, acct: &Account) -> Result<String, String> {
        if let Some(f) = self.db.folder_by_special(acct.id, "archive")? {
            return Ok(f);
        }
        if let Some(f) = self.db.folder_by_special(acct.id, "all")? {
            return Ok(f);
        }
        self.with_imap(acct, |c| c.sess.create("Archive"))?;
        self.refresh_folders(acct)?;
        Ok(self.db.folder_by_special(acct.id, "archive")?.unwrap_or_else(|| "Archive".into()))
    }

    pub fn archive(&self, acct: &Account, folder: &str, uids: &[u32]) -> Result<String, String> {
        let target = self.archive_folder(acct)?;
        if target != folder {
            self.move_to(acct, folder, uids, &target)?;
        }
        Ok(target)
    }

    // ------------------------------------------------------------------ push (IDLE)

    /// Syncs a folder and returns unread messages that arrived since the last sync.
    pub fn sync_collect_new(&self, acct: &Account, folder: &str, limit: u32) -> Result<Vec<MsgSummary>, String> {
        let before = self.db.max_uid(acct.id, folder)?;
        self.sync_folder(acct, folder, limit)?;
        if before == 0 {
            return Ok(vec![]); // first sync: don't announce old mail
        }
        self.db.new_since(acct.id, folder, before)
    }

    /// Opens a dedicated connection and waits (IMAP IDLE) until the folder changes or `timeout` passes.
    /// Returns Ok(true) when something changed.
    pub fn idle_wait(&self, conn: &mut Option<Conn>, acct: &Account, folder: &str, timeout: Duration) -> Result<bool, String> {
        if conn.is_none() {
            let auth = self.auth_for(acct)?;
            let mut c = connect(acct, &auth)?;
            c.sess.select(folder).map_err(|e| e.to_string())?;
            *conn = Some(c);
        }
        let c = conn.as_mut().unwrap();
        let res = (|| -> imap::Result<bool> {
            // NOOP flushes anything that arrived between IDLE sessions
            c.sess.noop()?;
            let mut changed = false;
            while c.sess.unsolicited_responses.try_recv().is_ok() {
                changed = true;
            }
            if changed {
                return Ok(true);
            }
            let handle = c.sess.idle()?;
            let outcome = handle.wait_with_timeout(timeout)?;
            Ok(matches!(outcome, imap::extensions::idle::WaitOutcome::MailboxChanged))
        })();
        match res {
            Ok(v) => Ok(v),
            Err(e) => {
                *conn = None;
                Err(format!("IDLE: {e}"))
            }
        }
    }

    // ------------------------------------------------------------------ backup / restore

    /// Downloads every message of every folder into a .zip (one mboxrd file per folder + manifest.json).
    pub fn backup(&self, acct: &Account, path: &std::path::Path, progress: &dyn Fn(Progress)) -> Result<u64, String> {
        use std::io::Write;
        let folders = self.refresh_folders(acct)?;
        let auth = self.auth_for(acct)?;
        let mut conn = connect(acct, &auth)?;
        let file = std::fs::File::create(path).map_err(|e| format!("Cannot create backup file: {e}"))?;
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut manifest = Vec::new();
        let mut total_msgs = 0u64;

        for (i, f) in folders.iter().filter(|f| !f.noselect).enumerate() {
            let mb = conn.sess.select(&f.name).map_err(|e| format!("{}: {e}", f.display))?;
            let mut uids: Vec<u32> = if mb.exists == 0 { vec![] } else { conn.sess.uid_search("ALL").map_err(|e| e.to_string())?.into_iter().collect() };
            uids.sort_unstable();
            let file_name = format!("{:03}-{}.mbox", i + 1, safe_name(&mime::decode_mutf7(&f.name)));
            zip.start_file(file_name.as_str(), opts).map_err(|e| e.to_string())?;
            let total = uids.len() as u64;
            progress(Progress { phase: "backup".into(), folder: f.display.clone(), done: 0, total });
            let mut done = 0u64;
            for chunk in uids.chunks(25) {
                let fetches = conn.sess.uid_fetch(uid_set(chunk), "(UID FLAGS INTERNALDATE BODY.PEEK[])").map_err(|e| e.to_string())?;
                for fe in fetches.iter() {
                    let Some(body) = fe.body() else { continue };
                    let flags: Vec<String> = fe.flags().iter().filter_map(flag_name).collect();
                    let date = fe.internal_date().map(|d| d.format("%a %b %e %H:%M:%S %Y").to_string()).unwrap_or_else(|| "Thu Jan  1 00:00:00 1970".into());
                    zip.write_all(mbox_entry(body, &flags, &date).as_slice()).map_err(|e| e.to_string())?;
                    done += 1;
                }
                progress(Progress { phase: "backup".into(), folder: f.display.clone(), done, total });
            }
            total_msgs += done;
            manifest.push(serde_json::json!({ "file": file_name, "imap_name": f.name, "display": f.display, "special": f.special, "count": done }));
        }
        let _ = conn.sess.logout();
        zip.start_file("manifest.json", opts).map_err(|e| e.to_string())?;
        let m = serde_json::json!({ "app": "Kite Mail", "version": 1, "account": acct.email, "created": chrono::Utc::now().to_rfc3339(), "folders": manifest });
        zip.write_all(serde_json::to_string_pretty(&m).unwrap().as_bytes()).map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())?;
        progress(Progress { phase: "done".into(), folder: String::new(), done: total_msgs, total: total_msgs });
        Ok(total_msgs)
    }

    /// Uploads a Kite Mail backup into this account. Folders are matched by role (Inbox, Sent...)
    /// or name and created when missing. Messages already present (same Message-ID) are skipped.
    pub fn restore(&self, acct: &Account, path: &std::path::Path, progress: &dyn Fn(Progress)) -> Result<(u64, u64), String> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|e| format!("Cannot open backup: {e}"))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Not a valid backup file: {e}"))?;
        let manifest: serde_json::Value = {
            let mut s = String::new();
            zip.by_name("manifest.json").map_err(|_| "manifest.json missing - not a Kite Mail backup")?.read_to_string(&mut s).map_err(|e| e.to_string())?;
            serde_json::from_str(&s).map_err(|e| e.to_string())?
        };
        let folders = self.refresh_folders(acct)?;
        let auth = self.auth_for(acct)?;
        let mut conn = connect(acct, &auth)?;
        let (mut added, mut skipped) = (0u64, 0u64);

        for entry in manifest["folders"].as_array().cloned().unwrap_or_default() {
            let special = entry["special"].as_str().unwrap_or("");
            let imap_name = entry["imap_name"].as_str().unwrap_or("INBOX").to_string();
            let display = entry["display"].as_str().unwrap_or(&imap_name).to_string();
            let target = folders.iter().find(|f| !special.is_empty() && f.special == special).map(|f| f.name.clone())
                .or_else(|| folders.iter().find(|f| f.name == imap_name).map(|f| f.name.clone()))
                .unwrap_or_else(|| {
                    let _ = conn.sess.create(&imap_name);
                    imap_name.clone()
                });
            let mut data = Vec::new();
            zip.by_name(entry["file"].as_str().unwrap_or("")).map_err(|e| e.to_string())?.read_to_end(&mut data).map_err(|e| e.to_string())?;
            let messages = parse_mbox(&data);
            let total = messages.len() as u64;
            conn.sess.select(&target).map_err(|e| format!("{display}: {e}"))?;
            for (i, (raw, flags)) in messages.iter().enumerate() {
                let mid = mail_parser::MessageParser::default().parse(raw.as_slice()).and_then(|m| m.message_id().map(String::from));
                let exists = match &mid {
                    Some(id) => conn.sess.uid_search(format!("HEADER Message-ID \"{}\"", id.replace('"', ""))).map(|r| !r.is_empty()).unwrap_or(false),
                    None => false,
                };
                if exists {
                    skipped += 1;
                } else {
                    let fl: Vec<Flag> = flags.iter().filter_map(|f| parse_flag(f)).collect();
                    conn.sess.append_with_flags(&target, raw, &fl).map_err(|e| format!("{display}: {e}"))?;
                    added += 1;
                }
                if i % 10 == 9 || i + 1 == messages.len() {
                    progress(Progress { phase: "restore".into(), folder: display.clone(), done: i as u64 + 1, total });
                }
            }
        }
        let _ = conn.sess.logout();
        self.refresh_folders(acct)?;
        progress(Progress { phase: "done".into(), folder: String::new(), done: added, total: added + skipped });
        Ok((added, skipped))
    }
}

fn clone_draft(d: &Draft) -> Draft {
    Draft {
        account_id: d.account_id, to: d.to.clone(), cc: d.cc.clone(), bcc: d.bcc.clone(), subject: d.subject.clone(),
        body: d.body.clone(), in_reply_to: d.in_reply_to.clone(), references: d.references.clone(),
        attachments: d.attachments.clone(), forward_from: d.forward_from.clone(), reply_to_msg: d.reply_to_msg.clone(),
        draft: d.draft.clone(), local_draft_id: d.local_draft_id, message_id: d.message_id.clone(), link_large: d.link_large,
    }
}

pub fn new_message_id() -> String {
    use rand::Rng;
    let r: u128 = rand::thread_rng().gen();
    format!("<{:032x}@kitemail>", r)
}

fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect::<String>().trim().to_string()
}

fn flag_name(f: &Flag) -> Option<String> {
    Some(match f {
        Flag::Seen => "\\Seen".into(),
        Flag::Answered => "\\Answered".into(),
        Flag::Flagged => "\\Flagged".into(),
        Flag::Draft => "\\Draft".into(),
        Flag::Custom(c) if !c.starts_with('\\') => c.to_string(),
        _ => return None,
    })
}

fn parse_flag(s: &str) -> Option<Flag<'static>> {
    Some(match s {
        "\\Seen" => Flag::Seen,
        "\\Answered" => Flag::Answered,
        "\\Flagged" => Flag::Flagged,
        "\\Draft" => Flag::Draft,
        "" => return None,
        other if !other.starts_with('\\') => Flag::Custom(std::borrow::Cow::Owned(other.to_string())),
        _ => return None,
    })
}

const FLAGS_HEADER: &str = "X-KiteMail-Flags: ";

/// One mboxrd entry: "From " line, flags header, body with LF endings and ">From" escaping.
pub fn mbox_entry(raw: &[u8], flags: &[String], date: &str) -> Vec<u8> {
    let mut out = format!("From kitemail@localhost {date}\n{FLAGS_HEADER}{}\n", flags.join(" ")).into_bytes();
    let text = raw.to_vec();
    for line in text.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let trimmed: &[u8] = { let mut i = 0; while i < line.len() && line[i] == b'>' { i += 1 } &line[i..] };
        if trimmed.starts_with(b"From ") {
            out.push(b'>');
        }
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    if !out.ends_with(b"\n\n") {
        out.push(b'\n');
    }
    out
}

/// Parses mboxrd data back into (raw message with CRLF endings, flags).
pub fn parse_mbox(data: &[u8]) -> Vec<(Vec<u8>, Vec<String>)> {
    let mut out: Vec<(Vec<u8>, Vec<String>)> = Vec::new();
    let mut cur: Option<(Vec<u8>, Vec<String>)> = None;
    let mut prev_blank = true;
    for line in data.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if prev_blank && line.starts_with(b"From ") {
            if let Some(m) = cur.take() { out.push(m); }
            cur = Some((Vec::new(), Vec::new()));
            prev_blank = false;
            continue;
        }
        if let Some((body, flags)) = cur.as_mut() {
            if body.is_empty() && flags.is_empty() && line.starts_with(FLAGS_HEADER.as_bytes()) {
                flags.extend(String::from_utf8_lossy(&line[FLAGS_HEADER.len()..]).split_whitespace().map(String::from));
                flags.push(String::new()); // marker: flags header consumed
            } else {
                let unesc = { let mut i = 0; while i < line.len() && line[i] == b'>' { i += 1 } i > 0 && line[i..].starts_with(b"From ") };
                body.extend_from_slice(if unesc { &line[1..] } else { line });
                body.extend_from_slice(b"\r\n");
            }
        }
        prev_blank = line.is_empty();
    }
    if let Some(m) = cur.take() { out.push(m); }
    for (body, flags) in out.iter_mut() {
        flags.retain(|f| !f.is_empty());
        // drop the blank separator line(s) mbox adds after each message
        while body.ends_with(b"\r\n\r\n") { body.truncate(body.len() - 2); }
    }
    out
}

// ---------------------------------------------------------------------- helpers

struct XOAuth2<'a> {
    user: &'a str,
    token: &'a str,
}

impl imap::Authenticator for XOAuth2<'_> {
    type Response = String;
    fn process(&self, _challenge: &[u8]) -> String {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}

pub fn connect(acct: &Account, auth: &Auth) -> Result<Conn, String> {
    let addr = (acct.imap_host.as_str(), acct.imap_port)
        .to_socket_addrs()
        .map_err(|e| format!("Cannot resolve {}: {e}", acct.imap_host))?
        .next()
        .ok_or_else(|| format!("Cannot resolve {}", acct.imap_host))?;
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(20))
        .map_err(|e| format!("Cannot connect to {}:{}: {e}", acct.imap_host, acct.imap_port))?;
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(120)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(60)));
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    let stream = tls.connect(&acct.imap_host, tcp).map_err(|e| format!("TLS error: {e}"))?;
    let mut client = imap::Client::new(stream);
    client.read_greeting().map_err(|e| format!("IMAP greeting failed: {e}"))?;

    let mut sess = match auth {
        Auth::Password(p) => client.login(&acct.username, p).map_err(|(e, _)| format!("IMAP login failed: {e}"))?,
        Auth::Bearer(t) => client
            .authenticate("XOAUTH2", &XOAuth2 { user: &acct.username, token: t })
            .map_err(|(e, _)| format!("IMAP sign-in failed: {e}. For Microsoft 365, check IMAP is enabled for the mailbox."))?,
    };
    let has_move = sess.capabilities().map(|c| c.has_str("MOVE")).unwrap_or(false);
    Ok(Conn { sess, selected: None, has_move })
}

fn ensure_selected(c: &mut Conn, folder: &str) -> imap::Result<()> {
    if c.selected.as_deref() != Some(folder) {
        c.sess.select(folder)?;
        c.selected = Some(folder.to_string());
    }
    Ok(())
}

fn flag_bits(flags: &[Flag]) -> (bool, bool, bool) {
    let mut r = (false, false, false);
    for f in flags {
        match f {
            Flag::Seen => r.0 = true,
            Flag::Flagged => r.1 = true,
            Flag::Answered => r.2 = true,
            _ => {}
        }
    }
    r
}

/// Compresses UIDs into an IMAP sequence set, e.g. [1,2,3,7,9,10] -> "1:3,7,9:10".
pub fn uid_set(uids: &[u32]) -> String {
    let mut v = uids.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut parts = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let start = v[i];
        let mut end = start;
        while i + 1 < v.len() && v[i + 1] == end + 1 {
            i += 1;
            end = v[i];
        }
        parts.push(if start == end { start.to_string() } else { format!("{start}:{end}") });
        i += 1;
    }
    parts.join(",")
}

fn special_of(name: &str, attrs: &[String]) -> &'static str {
    let has = |a: &str| attrs.iter().any(|x| x.eq_ignore_ascii_case(a));
    if name.eq_ignore_ascii_case("INBOX") {
        return "inbox";
    }
    if has("\\Sent") { return "sent"; }
    if has("\\Drafts") { return "drafts"; }
    if has("\\Trash") { return "trash"; }
    if has("\\Junk") { return "junk"; }
    if has("\\Archive") { return "archive"; }
    if has("\\All") { return "all"; }
    // Fallback for servers without SPECIAL-USE
    let leaf = name.rsplit(['/', '.']).next().unwrap_or(name).to_ascii_lowercase();
    match leaf.as_str() {
        "sent" | "sent items" | "sent messages" | "sent mail" => "sent",
        "drafts" | "draft" => "drafts",
        "trash" | "deleted items" | "deleted messages" | "bin" => "trash",
        "junk" | "spam" | "junk email" | "junk e-mail" | "bulk mail" => "junk",
        "archive" | "archives" => "archive",
        _ => "",
    }
}

const SPECIAL_ORDER: [&str; 7] = ["inbox", "drafts", "sent", "archive", "all", "junk", "trash"];

/// Turns raw LIST results into a display-ordered folder list: special folders first, then a tree.
pub fn organize_folders(account_id: i64, raw: Vec<(String, String, Vec<String>)>) -> Vec<(Folder, i64)> {
    let mut special: Vec<Folder> = Vec::new();
    let mut other: Vec<Folder> = Vec::new();
    let mut taken = HashSet::new();
    for (name, delim, attrs) in raw {
        let mut sp = special_of(&name, &attrs);
        if !sp.is_empty() && !taken.insert(sp) {
            sp = ""; // only one folder per special role
        }
        let noselect = attrs.iter().any(|a| a.eq_ignore_ascii_case("\\Noselect") || a.eq_ignore_ascii_case("\\NonExistent"));
        let decoded = mime::decode_mutf7(&name);
        let parts: Vec<&str> = if delim.is_empty() { vec![decoded.as_str()] } else { decoded.split(delim.as_str()).collect() };
        let leaf = parts.last().copied().unwrap_or(&decoded).to_string();
        let f = Folder {
            account_id,
            display: if sp == "inbox" { "Inbox".into() } else { leaf },
            special: sp.to_string(),
            noselect,
            depth: if sp.is_empty() { (parts.len() as u32).saturating_sub(1) } else { 0 },
            unread: 0,
            name,
        };
        if sp.is_empty() { other.push(f) } else { special.push(f) }
    }
    special.sort_by_key(|f| SPECIAL_ORDER.iter().position(|s| *s == f.special).unwrap_or(99));
    other.sort_by(|a, b| mime::decode_mutf7(&a.name).to_lowercase().cmp(&mime::decode_mutf7(&b.name).to_lowercase()));
    special.into_iter().chain(other).enumerate().map(|(i, f)| (f, i as i64)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uid_sets() {
        assert_eq!(uid_set(&[10, 9, 1, 2, 3, 7]), "1:3,7,9:10");
        assert_eq!(uid_set(&[5]), "5");
    }

    #[test]
    fn folder_order_and_specials() {
        let raw = vec![
            ("[Gmail]".into(), "/".into(), vec!["\\Noselect".into()]),
            ("[Gmail]/Sent Mail".into(), "/".into(), vec!["\\Sent".into()]),
            ("Projects/TLR".into(), "/".into(), vec![]),
            ("INBOX".into(), "/".into(), vec![]),
            ("Deleted Items".into(), "/".into(), vec![]),
            ("Projects".into(), "/".into(), vec![]),
        ];
        let f = organize_folders(1, raw);
        let names: Vec<_> = f.iter().map(|(f, _)| (f.display.as_str(), f.special.as_str(), f.depth)).collect();
        assert_eq!(names, vec![
            ("Inbox", "inbox", 0),
            ("Sent Mail", "sent", 0),
            ("Deleted Items", "trash", 0),
            ("[Gmail]", "", 0),
            ("Projects", "", 0),
            ("TLR", "", 1),
        ]);
        assert!(f[3].0.noselect);
    }

    #[test]
    fn mbox_roundtrip() {
        let m1 = b"Subject: a\r\n\r\nFrom here on\r\n>From quoted\r\nend\r\n".to_vec();
        let m2 = b"Subject: b\r\n\r\nsecond\r\n".to_vec();
        let mut data = mbox_entry(&m1, &["\\Seen".into(), "\\Flagged".into()], "Mon Nov 16 10:00:00 2026");
        data.extend(mbox_entry(&m2, &[], "Mon Nov 16 10:00:00 2026"));
        let back = parse_mbox(&data);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].0, m1);
        assert_eq!(back[0].1, vec!["\\Seen", "\\Flagged"]);
        assert_eq!(back[1].0, m2);
        assert!(back[1].1.is_empty());
    }
}
