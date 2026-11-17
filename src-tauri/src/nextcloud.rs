//! Nextcloud integration: two-way contacts (CardDAV) and tasks (CalDAV) sync, backups to a
//! Nextcloud folder (WebDAV) with retention, and large attachments as public share links (OCS).
//! Also works with other DAV servers when explicit collection URLs are configured.

use crate::dav::{self, Dav, PutError};
use crate::db::{new_uid, Db, SyncRow};
use crate::secrets;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PASSWORD_KEY: &str = "nextcloud-app-password";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct NcConfig {
    pub enabled: bool,
    /// e.g. https://cloud.example.com  (or a sub-path install https://example.com/nextcloud)
    pub server: String,
    pub user: String,
    pub sync_contacts: bool,
    pub sync_tasks: bool,
    /// Explicit collection URLs (optional – discovered automatically on Nextcloud).
    pub contacts_url: String,
    pub tasks_url: String,
    pub backup_schedule: String, // off | daily | weekly
    pub backup_folder: String,
    pub backup_keep: u32,
    pub link_threshold_mb: u32,
    pub link_expire_days: u32,
    pub last_sync: i64,
    pub last_error: String,
    pub last_backup: HashMap<String, i64>,
}

impl Default for NcConfig {
    fn default() -> Self {
        NcConfig {
            enabled: false,
            server: String::new(),
            user: String::new(),
            sync_contacts: true,
            sync_tasks: true,
            contacts_url: String::new(),
            tasks_url: String::new(),
            backup_schedule: "weekly".into(),
            backup_folder: "Kite Mail Backups".into(),
            backup_keep: 5,
            link_threshold_mb: 10,
            link_expire_days: 30,
            last_sync: 0,
            last_error: String::new(),
            last_backup: HashMap::new(),
        }
    }
}

type R<T> = Result<T, String>;

pub fn load(db: &Db) -> NcConfig {
    db.get_setting("nextcloud").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(db: &Db, cfg: &NcConfig) -> R<()> {
    db.set_setting("nextcloud", &serde_json::to_string(cfg).map_err(|e| e.to_string())?)
}

fn base(cfg: &NcConfig) -> String {
    let s = cfg.server.trim().trim_end_matches('/');
    if s.starts_with("http://") || s.starts_with("https://") { s.to_string() } else { format!("https://{s}") }
}

pub fn client(db: &Db, cfg: &NcConfig) -> R<Dav> {
    if !cfg.enabled || cfg.server.is_empty() || cfg.user.is_empty() {
        return Err("Nextcloud is not set up".into());
    }
    let pw = secrets::get(db, PASSWORD_KEY).ok_or("Nextcloud app password missing – reconnect in Settings")?;
    Ok(Dav::new(&cfg.user, &pw))
}

fn files_root(cfg: &NcConfig) -> String {
    format!("{}/remote.php/dav/files/{}", base(cfg), dav::enc(&cfg.user))
}

/// Verifies the login and returns the server's display of the user's files root.
pub fn test_login(cfg: &NcConfig, password: &str) -> R<()> {
    let d = Dav::new(&cfg.user, password);
    let explicit = [&cfg.contacts_url, &cfg.tasks_url].into_iter().find(|u| !u.is_empty()).cloned();
    match explicit {
        Some(u) if cfg.server.is_empty() => d.ping(&u),
        _ => d.ping(&format!("{}/", files_root(cfg))),
    }
}

/// Addressbook URL: configured, or the user's "contacts" book (first one otherwise).
fn contacts_collection(d: &Dav, cfg: &NcConfig) -> R<String> {
    if !cfg.contacts_url.is_empty() {
        return Ok(cfg.contacts_url.clone());
    }
    let home = format!("{}/remote.php/dav/addressbooks/users/{}/", base(cfg), dav::enc(&cfg.user));
    let books = d.collections(&home, "addressbook", false)?;
    let pick = books.iter().find(|b| b.href.trim_end_matches('/').ends_with("/contacts")).or(books.first()).ok_or("No address book found in Nextcloud")?;
    Ok(dav::resolve(&home, &pick.href))
}

/// Task list URL: configured, or a calendar named like "Tasks", else the first one supporting tasks.
fn tasks_collection(d: &Dav, cfg: &NcConfig) -> R<String> {
    if !cfg.tasks_url.is_empty() {
        return Ok(cfg.tasks_url.clone());
    }
    let home = format!("{}/remote.php/dav/calendars/{}/", base(cfg), dav::enc(&cfg.user));
    let cals = d.collections(&home, "calendar", true)?;
    let pick = cals.iter().find(|c| c.name.to_lowercase().contains("task")).or(cals.first()).ok_or("No task list found – create one in Nextcloud Tasks")?;
    Ok(dav::resolve(&home, &pick.href))
}

/// Lists task lists for the settings dropdown.
pub fn task_lists(db: &Db, cfg: &NcConfig) -> R<Vec<(String, String)>> {
    let d = client(db, cfg)?;
    let home = format!("{}/remote.php/dav/calendars/{}/", base(cfg), dav::enc(&cfg.user));
    Ok(d.collections(&home, "calendar", true)?.into_iter().map(|c| (dav::resolve(&home, &c.href), if c.name.is_empty() { c.href } else { c.name })).collect())
}

#[derive(Serialize, Default, Debug, Clone)]
pub struct SyncReport {
    pub contacts_down: u32,
    pub contacts_up: u32,
    pub tasks_down: u32,
    pub tasks_up: u32,
    pub deleted: u32,
}

/// One kind of syncable item (contacts or tasks).
trait Item: Sized {
    const TABLE: &'static str;
    const CONTENT_TYPE: &'static str;
    const EXT: &'static str;
    const CALDAV: bool;
    fn parse(raw: &str) -> Option<(Self, String)>;
    fn build(&self, uid: &str, prev: Option<&str>) -> String;
    fn load(db: &Db, id: i64) -> R<Option<Self>>;
    fn apply(db: &Db, id: Option<i64>, item: &Self, uid: &str, href: &str, etag: &str, raw: &str) -> R<()>;
}

impl Item for crate::models::Contact {
    const TABLE: &'static str = "contacts";
    const CONTENT_TYPE: &'static str = "text/vcard; charset=utf-8";
    const EXT: &'static str = "vcf";
    const CALDAV: bool = false;
    fn parse(raw: &str) -> Option<(Self, String)> { dav::vcard_to_contact(raw) }
    fn build(&self, uid: &str, prev: Option<&str>) -> String { dav::contact_to_vcard(self, uid, prev) }
    fn load(db: &Db, id: i64) -> R<Option<Self>> { db.contact_by_id(id) }
    fn apply(db: &Db, id: Option<i64>, item: &Self, uid: &str, href: &str, etag: &str, raw: &str) -> R<()> {
        db.apply_remote_contact(id, item, uid, href, etag, raw)
    }
}

impl Item for crate::models::Todo {
    const TABLE: &'static str = "todos";
    const CONTENT_TYPE: &'static str = "text/calendar; charset=utf-8";
    const EXT: &'static str = "ics";
    const CALDAV: bool = true;
    fn parse(raw: &str) -> Option<(Self, String)> { dav::ical_to_todo(raw) }
    fn build(&self, uid: &str, prev: Option<&str>) -> String { dav::todo_to_ical(self, uid, prev) }
    fn load(db: &Db, id: i64) -> R<Option<Self>> { db.todo_by_id(id) }
    fn apply(db: &Db, id: Option<i64>, item: &Self, uid: &str, href: &str, etag: &str, raw: &str) -> R<()> {
        db.apply_remote_todo(id, item, uid, href, etag, raw)
    }
}

/// Two-way sync of one collection. Server wins on conflicting edits; deletions propagate both ways.
fn sync_items<T: Item>(db: &Db, d: &Dav, collection: &str) -> R<(u32, u32, u32)> {
    let (mut down, mut up, mut deleted) = (0u32, 0u32, 0u32);
    let remote = if T::CALDAV { d.list_todos(collection)? } else { d.list(collection)? };
    let remote_by_href: HashMap<String, String> = remote.iter().map(|r| (r.href.clone(), r.etag.clone())).collect();
    let rows = db.sync_rows(T::TABLE)?;
    let local_by_href: HashMap<String, SyncRow> = rows.iter().filter(|r| !r.href.is_empty()).map(|r| (r.href.clone(), r.clone())).collect();

    // 1. local deletions -> server
    for r in rows.iter().filter(|r| r.deleted) {
        if !r.href.is_empty() && remote_by_href.contains_key(&r.href) {
            d.delete(&dav::resolve(collection, &r.href), Some(&r.etag))?;
        }
        db.purge(T::TABLE, r.id)?;
        deleted += 1;
    }

    // 2. server deletions -> local (only for rows we haven't changed locally)
    for r in rows.iter().filter(|r| !r.deleted && !r.href.is_empty() && !remote_by_href.contains_key(&r.href)) {
        if r.dirty {
            // edited here but deleted there: recreate on server
            continue;
        }
        db.purge(T::TABLE, r.id)?;
        deleted += 1;
    }

    // 3. new or changed on server -> local
    let to_fetch: Vec<String> = remote
        .iter()
        .filter(|res| match local_by_href.get(&res.href) {
            None => true,
            Some(l) => !l.deleted && l.etag != res.etag,
        })
        .map(|res| res.href.clone())
        .collect();
    if !to_fetch.is_empty() {
        let fetched = d.multiget(collection, &to_fetch, T::CALDAV)?;
        for (href, etag, raw) in fetched {
            let Some((item, mut uid)) = T::parse(&raw) else { continue };
            if uid.is_empty() {
                uid = new_uid();
            }
            let etag = if etag.is_empty() { remote_by_href.get(&href).cloned().unwrap_or_default() } else { etag };
            let existing = local_by_href.get(&href).map(|l| l.id).or_else(|| rows.iter().find(|r| r.href.is_empty() && r.uid == uid).map(|r| r.id));
            T::apply(db, existing, &item, &uid, &href, &etag, &raw)?;
            down += 1;
        }
    }

    // 4. local new / changed -> server
    let rows = db.sync_rows(T::TABLE)?;
    for r in rows.iter().filter(|r| r.dirty && !r.deleted) {
        let Some(item) = T::load(db, r.id)? else { continue };
        let uid = if r.uid.is_empty() { new_uid() } else { r.uid.clone() };
        let (url, href, if_match) = if !r.href.is_empty() && remote_by_href.contains_key(&r.href) {
            (dav::resolve(collection, &r.href), r.href.clone(), Some(r.etag.as_str()))
        } else {
            let url = format!("{}/{}.{}", collection.trim_end_matches('/'), dav::enc(&uid), T::EXT);
            let href = url::Url::parse(&url).map(|u| dav::url_decode(u.path())).unwrap_or_default();
            (url, href, None)
        };
        let raw = item.build(&uid, if r.raw.is_empty() { None } else { Some(&r.raw) });
        match d.put(&url, T::CONTENT_TYPE, &raw, if_match) {
            Ok(etag) => {
                let etag = match etag { Some(e) => e, None => d.etag_of(&url).unwrap_or_default() };
                db.mark_synced(T::TABLE, r.id, &uid, &href, &etag, &raw)?;
                up += 1;
            }
            Err(PutError::Conflict) => {
                // changed on the server meanwhile: server wins
                let (raw, etag) = d.get(&url)?;
                if let Some((item, uid2)) = T::parse(&raw) {
                    T::apply(db, Some(r.id), &item, if uid2.is_empty() { &uid } else { &uid2 }, &href, &etag, &raw)?;
                    down += 1;
                }
            }
            Err(PutError::Other(e)) => return Err(e),
        }
    }
    Ok((down, up, deleted))
}

/// Runs contacts and/or tasks sync according to the configuration.
pub fn sync(db: &Db) -> R<SyncReport> {
    let mut cfg = load(db);
    let d = client(db, &cfg)?;
    db.ensure_uids()?;
    let mut rep = SyncReport::default();
    let res = (|| -> R<()> {
        if cfg.sync_contacts {
            let url = contacts_collection(&d, &cfg)?;
            let (dn, upn, del) = sync_items::<crate::models::Contact>(db, &d, &url)?;
            rep.contacts_down = dn;
            rep.contacts_up = upn;
            rep.deleted += del;
        }
        if cfg.sync_tasks {
            let url = tasks_collection(&d, &cfg)?;
            let (dn, upn, del) = sync_items::<crate::models::Todo>(db, &d, &url)?;
            rep.tasks_down = dn;
            rep.tasks_up = upn;
            rep.deleted += del;
        }
        Ok(())
    })();
    cfg = load(db);
    cfg.last_sync = chrono::Utc::now().timestamp();
    cfg.last_error = res.as_ref().err().cloned().unwrap_or_default();
    save(db, &cfg)?;
    res.map(|_| rep)
}

// ============================================================ backups

/// Uploads a finished backup file and removes old ones beyond `backup_keep`.
pub fn upload_backup(db: &Db, local: &std::path::Path, account_email: &str) -> R<String> {
    let cfg = load(db);
    let d = client(db, &cfg)?;
    let root = files_root(&cfg);
    let folder = format!("{}/{}", cfg.backup_folder.trim_matches('/'), account_email);
    d.mkcol_all(&root, &folder)?;
    let name = local.file_name().and_then(|n| n.to_str()).unwrap_or("backup.zip");
    let dir_url = format!("{root}/{}", folder.split('/').map(dav::enc).collect::<Vec<_>>().join("/"));
    d.put_file(&format!("{dir_url}/{}", dav::enc(name)), local)?;
    // retention: names are dated (…-YYYY-MM-DD.zip) so a name sort is chronological
    if cfg.backup_keep > 0 {
        // Work with file names inside the known folder: robust to servers that return relative hrefs.
        let mut files: Vec<String> = d
            .list_files(&format!("{dir_url}/"))?
            .into_iter()
            .filter_map(|(h, _)| h.trim_end_matches('/').rsplit('/').next().map(String::from))
            .filter(|n| n.starts_with("kitemail-backup-") && n.ends_with(".zip"))
            .collect();
        files.sort();
        let excess = files.len().saturating_sub(cfg.backup_keep as usize);
        for n in files.into_iter().take(excess) {
            d.delete(&format!("{dir_url}/{}", dav::enc(&n)), None)?;
        }
    }
    Ok(format!("{}/{}", folder, name))
}

pub fn backup_due(cfg: &NcConfig, email: &str) -> bool {
    let every = match cfg.backup_schedule.as_str() { "daily" => 86_400, "weekly" => 7 * 86_400, _ => return false };
    let last = cfg.last_backup.get(email).copied().unwrap_or(0);
    chrono::Utc::now().timestamp() - last >= every
}

pub fn mark_backup(db: &Db, email: &str) -> R<()> {
    let mut cfg = load(db);
    cfg.last_backup.insert(email.to_string(), chrono::Utc::now().timestamp());
    save(db, &cfg)
}

// ============================================================ large attachments as links

/// Uploads a file to "Kite Mail Attachments/YYYY-MM-DD/" and returns a public share link.
pub fn share_file(db: &Db, path: &std::path::Path) -> R<String> {
    let cfg = load(db);
    let d = client(db, &cfg)?;
    let root = files_root(&cfg);
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    let folder = format!("Kite Mail Attachments/{day}");
    d.mkcol_all(&root, &folder)?;
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("Bad file name")?;
    let url = format!("{root}/{}/{}", folder.split('/').map(dav::enc).collect::<Vec<_>>().join("/"), dav::enc(name));
    d.put_file(&url, path)?;
    let ocs = format!("{}/ocs/v2.php/apps/files_sharing/api/v1/shares?format=json", base(&cfg));
    let rel = format!("/{folder}/{name}");
    let mut form: Vec<(&str, String)> = vec![("path", rel), ("shareType", "3".into()), ("permissions", "1".into())];
    if cfg.link_expire_days > 0 {
        let exp = (chrono::Local::now() + chrono::Duration::days(cfg.link_expire_days as i64)).format("%Y-%m-%d").to_string();
        form.push(("expireDate", exp));
    }
    let form_ref: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let v = d.ocs_post(&ocs, &form_ref)?;
    v["ocs"]["data"]["url"].as_str().map(String::from).ok_or_else(|| {
        format!("Nextcloud did not return a link: {}", v["ocs"]["meta"]["message"].as_str().unwrap_or("unknown error"))
    })
}

/// Replaces large local attachments with share links appended to the body.
pub fn linkify_attachments(db: &Db, body: &mut String, paths: &mut Vec<String>) -> R<u32> {
    let cfg = load(db);
    let limit = cfg.link_threshold_mb.max(1) as u64 * 1024 * 1024;
    let mut links = Vec::new();
    let mut keep = Vec::new();
    for p in paths.drain(..) {
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        if size >= limit {
            let path = std::path::Path::new(&p);
            let url = share_file(db, path)?;
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            links.push(format!("📎 {name} ({:.1} MB): {url}", size as f64 / 1_048_576.0));
        } else {
            keep.push(p);
        }
    }
    *paths = keep;
    if !links.is_empty() {
        let exp = if cfg.link_expire_days > 0 { format!(" (links expire in {} days)", cfg.link_expire_days) } else { String::new() };
        let block = format!("\n\nFiles shared via Nextcloud{exp}:\n{}\n", links.join("\n"));
        // put links above the quoted original, if any
        match body.find("\n\nOn ").or_else(|| body.find("\n\n---------- Forwarded")) {
            Some(i) => body.insert_str(i, &block),
            None => body.push_str(&block),
        }
    }
    Ok(links.len() as u32)
}
