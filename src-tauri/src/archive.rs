//! Archive: mail imported from backups and other programs, kept apart from the mail accounts.
//!
//! Formats: Outlook PST, Outlook MSG, EML (and Apple EMLX), MBOX, Maildir, Kite Mail backups
//! (.zip) – single files or whole folders of them. PST and MSG are converted to standard
//! RFC 5322 messages on import, so everything afterwards (reader, reply, forward, copy back to
//! a mailbox) works the same way for every source.
//!
//! Stored in its own SQLite file (archive.db): deflate-compressed messages plus an FTS5
//! full-text index (subject, people, body).

use crate::mime;
use mail_parser::{MessageParser, MimeHeaders};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

type R<T> = Result<T, String>;

/// Set by "Cancel" while an import runs.
pub static CANCEL: AtomicBool = AtomicBool::new(false);

const BATCH: usize = 200;

#[derive(Serialize, Debug, Clone)]
pub struct ArchiveInfo {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub created: i64,
    pub count: i64,
    pub folders: Vec<FolderCount>,
}

#[derive(Serialize, Debug, Clone)]
pub struct FolderCount {
    pub name: String,
    pub count: i64,
}

#[derive(Serialize, Debug, Clone)]
pub struct ArchMsg {
    pub id: i64,
    pub archive_id: i64,
    pub folder: String,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub date: i64,
    pub size: i64,
    pub has_attach: bool,
    pub preview: String,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct ImportProgress {
    pub archive_id: i64,
    pub file: String,
    pub folder: String,
    pub added: u64,
    pub skipped: u64,
    pub failed: u64,
    pub finished: bool,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct ImportReport {
    pub archive_id: i64,
    pub name: String,
    pub added: u64,
    pub skipped: u64,
    pub failed: u64,
    pub cancelled: bool,
    pub errors: Vec<String>,
}

pub struct Archive {
    c: Mutex<Connection>,
}

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

impl Archive {
    pub fn open(path: &Path) -> R<Self> {
        Self::init(Connection::open(path).map_err(e)?)
    }

    #[cfg(test)]
    pub fn memory() -> Self {
        Self::init(Connection::open_in_memory().expect("memory db")).expect("schema")
    }

    fn init(c: Connection) -> R<Self> {
        c.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS archives(id INTEGER PRIMARY KEY, name TEXT NOT NULL, source TEXT NOT NULL DEFAULT '', created INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS amsg(
                id INTEGER PRIMARY KEY, archive_id INTEGER NOT NULL, folder TEXT NOT NULL, dkey TEXT NOT NULL,
                subject TEXT NOT NULL DEFAULT '', from_name TEXT NOT NULL DEFAULT '', from_addr TEXT NOT NULL DEFAULT '',
                to_addr TEXT NOT NULL DEFAULT '', date INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0,
                has_attach INTEGER NOT NULL DEFAULT 0, preview TEXT NOT NULL DEFAULT '', raw BLOB NOT NULL);
             CREATE INDEX IF NOT EXISTS amsg_list ON amsg(archive_id, folder, date);
             CREATE UNIQUE INDEX IF NOT EXISTS amsg_dedupe ON amsg(archive_id, dkey);
             CREATE VIRTUAL TABLE IF NOT EXISTS amsg_fts USING fts5(subject, people, body, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');",
        )
        .map_err(e)?;
        Ok(Self { c: Mutex::new(c) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.c.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn create(&self, name: &str, source: &str) -> R<i64> {
        let c = self.conn();
        c.execute("INSERT INTO archives(name, source, created) VALUES(?1, ?2, ?3)", params![name, source, chrono::Utc::now().timestamp()]).map_err(e)?;
        Ok(c.last_insert_rowid())
    }

    pub fn rename(&self, id: i64, name: &str) -> R<()> {
        self.conn().execute("UPDATE archives SET name=?2 WHERE id=?1", params![id, name.trim()]).map_err(e)?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> R<()> {
        let mut c = self.conn();
        let tx = c.transaction().map_err(e)?;
        tx.execute("DELETE FROM amsg_fts WHERE rowid IN (SELECT id FROM amsg WHERE archive_id=?1)", [id]).map_err(e)?;
        tx.execute("DELETE FROM amsg WHERE archive_id=?1", [id]).map_err(e)?;
        tx.execute("DELETE FROM archives WHERE id=?1", [id]).map_err(e)?;
        tx.commit().map_err(e)?;
        let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA incremental_vacuum;");
        Ok(())
    }

    pub fn list(&self) -> R<Vec<ArchiveInfo>> {
        let c = self.conn();
        let mut st = c.prepare("SELECT id, name, source, created FROM archives ORDER BY created DESC").map_err(e)?;
        let mut out: Vec<ArchiveInfo> = st
            .query_map([], |r| Ok(ArchiveInfo { id: r.get(0)?, name: r.get(1)?, source: r.get(2)?, created: r.get(3)?, count: 0, folders: vec![] }))
            .map_err(e)?
            .filter_map(Result::ok)
            .collect();
        let mut fq = c.prepare("SELECT folder, count(*) FROM amsg WHERE archive_id=?1 GROUP BY folder ORDER BY folder").map_err(e)?;
        for a in &mut out {
            a.folders = fq.query_map([a.id], |r| Ok(FolderCount { name: r.get(0)?, count: r.get(1)? })).map_err(e)?.filter_map(Result::ok).collect();
            a.count = a.folders.iter().map(|f| f.count).sum();
        }
        Ok(out)
    }

    const COLS: &'static str = "id, archive_id, folder, subject, from_name, from_addr, to_addr, date, size, has_attach, preview";

    fn row(r: &rusqlite::Row) -> rusqlite::Result<ArchMsg> {
        Ok(ArchMsg {
            id: r.get(0)?, archive_id: r.get(1)?, folder: r.get(2)?, subject: r.get(3)?, from_name: r.get(4)?, from_addr: r.get(5)?,
            to: r.get(6)?, date: r.get(7)?, size: r.get(8)?, has_attach: r.get::<_, i64>(9)? != 0, preview: r.get(10)?,
        })
    }

    pub fn messages(&self, archive_id: i64, folder: Option<&str>, offset: i64, limit: i64) -> R<Vec<ArchMsg>> {
        let c = self.conn();
        let sql = format!(
            "SELECT {} FROM amsg WHERE archive_id=?1 AND (?2 IS NULL OR folder=?2) ORDER BY date DESC LIMIT ?3 OFFSET ?4",
            Self::COLS
        );
        let mut st = c.prepare(&sql).map_err(e)?;
        let rows = st.query_map(params![archive_id, folder, limit, offset], Self::row).map_err(e)?.filter_map(Result::ok).collect();
        Ok(rows)
    }

    /// Full-text search (every word must match; prefixes allowed) across one or all archives.
    pub fn search(&self, archive_id: Option<i64>, query: &str, limit: i64) -> R<Vec<ArchMsg>> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| t.chars().filter(|c| c.is_alphanumeric() || matches!(c, '@' | '.' | '_' | '-')).collect::<String>())
            .filter(|t| !t.is_empty())
            .map(|t| format!("\"{}\"*", t.replace('"', "")))
            .collect();
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let c = self.conn();
        let sql = format!(
            "SELECT {} FROM amsg WHERE id IN (SELECT rowid FROM amsg_fts WHERE amsg_fts MATCH ?1) AND (?2 IS NULL OR archive_id=?2) ORDER BY date DESC LIMIT ?3",
            Self::COLS
        );
        let mut st = c.prepare(&sql).map_err(e)?;
        let rows = st.query_map(params![terms.join(" "), archive_id, limit], Self::row).map_err(e)?.filter_map(Result::ok).collect();
        Ok(rows)
    }

    pub fn get(&self, id: i64) -> R<(ArchMsg, Vec<u8>)> {
        let c = self.conn();
        let (m, blob): (ArchMsg, Vec<u8>) = c
            .query_row(&format!("SELECT {}, raw FROM amsg WHERE id=?1", Self::COLS), [id], |r| Ok((Self::row(r)?, r.get(11)?)))
            .optional()
            .map_err(e)?
            .ok_or("This archived message no longer exists")?;
        let mut raw = Vec::new();
        flate2::read::DeflateDecoder::new(&blob[..]).read_to_end(&mut raw).map_err(e)?;
        Ok((m, raw))
    }

    pub fn delete_messages(&self, ids: &[i64]) -> R<()> {
        let mut c = self.conn();
        let tx = c.transaction().map_err(e)?;
        for id in ids {
            tx.execute("DELETE FROM amsg_fts WHERE rowid=?1", [id]).map_err(e)?;
            tx.execute("DELETE FROM amsg WHERE id=?1", [id]).map_err(e)?;
        }
        tx.commit().map_err(e)
    }

    /// Inserts a batch; returns (added, duplicates).
    fn insert(&self, archive_id: i64, batch: &mut Vec<Indexed>) -> R<(u64, u64)> {
        if batch.is_empty() {
            return Ok((0, 0));
        }
        let mut c = self.conn();
        let tx = c.transaction().map_err(e)?;
        let (mut added, mut dup) = (0, 0);
        {
            let mut ins = tx
                .prepare("INSERT OR IGNORE INTO amsg(archive_id, folder, dkey, subject, from_name, from_addr, to_addr, date, size, has_attach, preview, raw) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)")
                .map_err(e)?;
            let mut fts = tx.prepare("INSERT INTO amsg_fts(rowid, subject, people, body) VALUES(?1,?2,?3,?4)").map_err(e)?;
            for m in batch.drain(..) {
                let n = ins
                    .execute(params![archive_id, m.folder, m.dkey, m.subject, m.from_name, m.from_addr, m.to, m.date, m.size, m.has_attach as i64, m.preview, m.blob])
                    .map_err(e)?;
                if n == 0 {
                    dup += 1;
                    continue;
                }
                fts.execute(params![tx.last_insert_rowid(), m.subject, m.people, m.body]).map_err(e)?;
                added += 1;
            }
        }
        tx.commit().map_err(e)?;
        Ok((added, dup))
    }
}

/// A parsed, compressed message ready to insert.
struct Indexed {
    folder: String,
    dkey: String,
    subject: String,
    from_name: String,
    from_addr: String,
    to: String,
    people: String,
    date: i64,
    size: i64,
    has_attach: bool,
    preview: String,
    body: String,
    blob: Vec<u8>,
}

fn index(folder: &str, raw: &[u8]) -> R<Indexed> {
    let m = MessageParser::default().parse(raw).ok_or("not an email message")?;
    if m.headers().is_empty() {
        return Err("not an email message".into());
    }
    let h = mime::parse_headers(raw);
    let cc = mime::format_address_list(m.cc());
    let body = mime::search_text(raw);
    let mid = m.message_id().unwrap_or("").trim().to_string();
    let dkey = if mid.is_empty() { format!("sha:{}", hex(&Sha256::digest(raw))) } else { format!("mid:{mid}") };
    let has_attach = m.attachments().any(|a| a.attachment_name().is_some() || !a.is_text());
    let mut enc = flate2::write::DeflateEncoder::new(Vec::with_capacity(raw.len() / 3), flate2::Compression::default());
    enc.write_all(raw).map_err(e)?;
    Ok(Indexed {
        folder: folder.to_string(),
        dkey,
        people: format!("{} {} {} {}", h.from_name, h.from_addr, h.to, cc),
        subject: h.subject,
        from_name: h.from_name,
        from_addr: h.from_addr,
        to: h.to,
        date: h.date,
        size: raw.len() as i64,
        has_attach,
        preview: mime::preview(&body),
        body: body.chars().take(50_000).collect(),
        blob: enc.finish().map_err(e)?,
    })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ============================================================ importing

/// Collects messages, writes them in batches and reports progress.
pub struct Importer<'a> {
    ar: &'a Archive,
    pub report: ImportReport,
    batch: Vec<Indexed>,
    file: String,
    folder: String,
    progress: &'a dyn Fn(&ImportProgress),
    last_emit: std::time::Instant,
}

impl<'a> Importer<'a> {
    pub fn new(ar: &'a Archive, archive_id: i64, name: &str, progress: &'a dyn Fn(&ImportProgress)) -> Self {
        CANCEL.store(false, Ordering::SeqCst);
        Self {
            ar,
            report: ImportReport { archive_id, name: name.to_string(), ..Default::default() },
            batch: Vec::new(),
            file: String::new(),
            folder: String::new(),
            progress,
            last_emit: std::time::Instant::now(),
        }
    }

    fn cancelled(&mut self) -> bool {
        if CANCEL.load(Ordering::SeqCst) {
            self.report.cancelled = true;
        }
        self.report.cancelled
    }

    fn fail(&mut self, what: &str, err: &str) {
        self.report.failed += 1;
        if self.report.errors.len() < 25 {
            self.report.errors.push(format!("{what}: {err}"));
        }
    }

    pub fn add(&mut self, folder: &str, raw: &[u8]) {
        self.folder = folder.to_string();
        match index(folder, raw) {
            Ok(ix) => self.batch.push(ix),
            Err(err) => {
                let what = format!("{}{}", self.file, if folder.is_empty() { String::new() } else { format!(" / {folder}") });
                self.fail(&what, &err)
            }
        }
        if self.batch.len() >= BATCH {
            self.flush();
        }
    }

    fn flush(&mut self) {
        match self.ar.insert(self.report.archive_id, &mut self.batch) {
            Ok((a, d)) => {
                self.report.added += a;
                self.report.skipped += d;
            }
            Err(err) => {
                let n = self.batch.len();
                self.batch.clear();
                self.report.failed += n as u64;
                if self.report.errors.len() < 25 {
                    self.report.errors.push(format!("Saving to the archive failed: {err}"));
                }
            }
        }
        if self.last_emit.elapsed().as_millis() > 250 {
            self.last_emit = std::time::Instant::now();
            (self.progress)(&self.progress_now(false));
        }
    }

    fn progress_now(&self, finished: bool) -> ImportProgress {
        ImportProgress {
            archive_id: self.report.archive_id,
            file: self.file.clone(),
            folder: self.folder.clone(),
            added: self.report.added,
            skipped: self.report.skipped,
            failed: self.report.failed,
            finished,
        }
    }

    pub fn finish(mut self) -> ImportReport {
        self.flush();
        (self.progress)(&self.progress_now(true));
        self.report
    }

    /// Imports a file or a folder (recursively). `base` is the folder name prefix in the archive.
    pub fn import_path(&mut self, path: &Path, base: &str) {
        if self.cancelled() {
            return;
        }
        self.file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (self.progress)(&self.progress_now(false));
        if path.is_dir() {
            if is_maildir(path) {
                return self.import_maildir(path, if base.is_empty() { "Inbox" } else { base });
            }
            let mut entries: Vec<PathBuf> = match std::fs::read_dir(path) {
                Ok(rd) => rd.flatten().map(|d| d.path()).collect(),
                Err(err) => return self.fail(&path.display().to_string(), &err.to_string()),
            };
            entries.sort();
            for p in entries {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if name.starts_with('.') && !is_maildir(&p) {
                    continue;
                }
                let sub = if p.is_dir() { join_folder(base, &name) } else { base.to_string() };
                self.import_path(&p, &sub);
                if self.cancelled() {
                    return;
                }
            }
            return;
        }
        let ext = path.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let folder = if base.is_empty() { "Imported".to_string() } else { base.to_string() };
        let res: R<()> = match ext.as_str() {
            "pst" => self.import_pst(path, base),
            "ost" => Err("OST files are Outlook's offline cache and can't be read – export a PST from Outlook instead (File → Open & Export → Import/Export)".into()),
            "msg" => std::fs::read(path).map_err(e).and_then(|b| msg_to_eml(&b)).map(|raw| self.add(&folder, &raw)),
            "eml" | "emlx" | "txt" => std::fs::read(path).map_err(e).map(|b| {
                let raw = if ext == "emlx" { emlx_payload(&b) } else { b };
                self.add(&folder, &raw)
            }),
            "zip" => self.import_kite_zip(path, base),
            "mbox" | "mbx" | "mbs" => self.import_mbox_file(path, &mbox_folder(path, base)),
            _ => {
                // mbox files often have no extension (Thunderbird "Inbox", "Sent")
                if starts_with_from(path) {
                    self.import_mbox_file(path, &mbox_folder(path, base))
                } else {
                    return; // unrelated file in a folder: ignore quietly
                }
            }
        };
        if let Err(err) = res {
            let what = self.file.clone();
            self.fail(&what, &err);
        }
    }

    fn import_mbox_file(&mut self, path: &Path, folder: &str) -> R<()> {
        let f = std::fs::File::open(path).map_err(e)?;
        self.import_mbox(BufReader::with_capacity(1 << 20, f), folder)
    }

    /// Streams an mbox (mboxo / mboxrd), so multi-GB files never sit in memory.
    pub fn import_mbox<Rd: BufRead>(&mut self, mut rd: Rd, folder: &str) -> R<()> {
        let mut cur: Option<Vec<u8>> = None;
        let mut prev_blank = true;
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = rd.read_until(b'\n', &mut line).map_err(e)?;
            if n == 0 {
                break;
            }
            if prev_blank && line.starts_with(b"From ") {
                if let Some(msg) = cur.take() {
                    self.add_mbox_msg(folder, msg);
                    if self.cancelled() {
                        return Ok(());
                    }
                }
                cur = Some(Vec::new());
                prev_blank = false;
                continue;
            }
            let blank = line == b"\n" || line == b"\r\n";
            if let Some(buf) = cur.as_mut() {
                // mboxrd: ">From " -> "From " (one level of quoting removed)
                let gt = line.iter().take_while(|&&b| b == b'>').count();
                if gt > 0 && line[gt..].starts_with(b"From ") {
                    buf.extend_from_slice(&line[1..]);
                } else {
                    buf.extend_from_slice(&line);
                }
            }
            prev_blank = blank;
        }
        if let Some(msg) = cur.take() {
            self.add_mbox_msg(folder, msg);
        }
        Ok(())
    }

    fn add_mbox_msg(&mut self, folder: &str, mut msg: Vec<u8>) {
        // drop the blank separator line before the next "From "
        while msg.ends_with(b"\n") {
            msg.pop();
            if msg.ends_with(b"\r") {
                msg.pop();
            }
        }
        msg.push(b'\n');
        // Kite Mail backups carry their flags in a private header – not part of the message
        if msg.starts_with(crate::mail::FLAGS_HEADER.as_bytes()) {
            if let Some(i) = msg.iter().position(|&b| b == b'\n') {
                msg.drain(..=i);
            }
        }
        self.add(folder, &msg);
    }

    fn import_maildir(&mut self, dir: &Path, folder: &str) {
        for sub in ["cur", "new"] {
            let Ok(rd) = std::fs::read_dir(dir.join(sub)) else { continue };
            let mut files: Vec<PathBuf> = rd.flatten().map(|d| d.path()).filter(|p| p.is_file()).collect();
            files.sort();
            for f in files {
                match std::fs::read(&f) {
                    Ok(b) => self.add(folder, &b),
                    Err(err) => self.fail(&f.display().to_string(), &err.to_string()),
                }
                if self.cancelled() {
                    return;
                }
            }
        }
        // Maildir++ sub-folders: ".Sent", ".Projects.TLR"
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut subs: Vec<PathBuf> = rd.flatten().map(|d| d.path()).filter(|p| p.is_dir() && is_maildir(p)).collect();
        subs.sort();
        for s in subs {
            let name = s.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(stripped) = name.strip_prefix('.') {
                let parent = if folder == "Inbox" { String::new() } else { folder.to_string() };
                let nested = join_folder(&parent, &stripped.replace('.', "/"));
                self.import_maildir(&s, &nested);
            }
        }
    }

    /// Kite Mail backup: a zip with one mbox per folder.
    fn import_kite_zip(&mut self, path: &Path, base: &str) -> R<()> {
        let f = std::fs::File::open(path).map_err(e)?;
        let mut z = zip::ZipArchive::new(f).map_err(|_| "This .zip is not a mail backup".to_string())?;
        let mut found = false;
        for i in 0..z.len() {
            let entry = z.by_index(i).map_err(e)?;
            let name = entry.name().map(|n| n.to_string()).unwrap_or_default();
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".mbox") {
                found = true;
                let folder = join_folder(base, name.trim_end_matches(".mbox").trim_end_matches(".MBOX"));
                self.import_mbox(BufReader::with_capacity(1 << 20, entry), &folder)?;
            } else if lower.ends_with(".eml") {
                found = true;
                let mut b = Vec::new();
                let mut entry = entry;
                entry.read_to_end(&mut b).map_err(e)?;
                let dir = name.rsplit_once('/').map(|(d, _)| d).unwrap_or("Imported");
                self.add(&join_folder(base, dir), &b);
            }
            if self.cancelled() {
                break;
            }
        }
        if found { Ok(()) } else { Err("No mailboxes (.mbox or .eml) found in this .zip".into()) }
    }

    fn import_pst(&mut self, path: &Path, base: &str) -> R<()> {
        let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let prefix = if base.is_empty() { String::new() } else { join_folder(base, &stem) };
        match pst::import_unicode(self, path, &prefix) {
            Ok(()) => Ok(()),
            Err(unicode_err) => pst::import_ansi(self, path, &prefix).map_err(|_| friendly_pst_error(&unicode_err)),
        }
    }
}

fn friendly_pst_error(err: &str) -> String {
    if err.contains("wMagic") || err.contains("Invalid HEADER") {
        "This doesn't look like an Outlook PST file (or it is password-protected / damaged)".into()
    } else {
        format!("Can't read this PST file: {err}")
    }
}

fn join_folder(base: &str, name: &str) -> String {
    let name = name.trim_matches('/');
    match (base.is_empty(), name.is_empty()) {
        (true, _) => name.to_string(),
        (_, true) => base.to_string(),
        _ => format!("{base}/{name}"),
    }
}

fn mbox_folder(path: &Path, base: &str) -> String {
    let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into());
    join_folder(base, &stem)
}

fn is_maildir(p: &Path) -> bool {
    p.join("cur").is_dir() && p.join("new").is_dir()
}

fn starts_with_from(p: &Path) -> bool {
    let mut b = [0u8; 5];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut b)).is_ok() && &b == b"From "
}

/// Apple Mail .emlx: "<byte count>\n<message><plist>"
fn emlx_payload(b: &[u8]) -> Vec<u8> {
    if let Some(nl) = b.iter().position(|&c| c == b'\n') {
        if let Ok(n) = std::str::from_utf8(&b[..nl]).unwrap_or("").trim().parse::<usize>() {
            let start = nl + 1;
            return b[start..(start + n).min(b.len())].to_vec();
        }
    }
    b.to_vec()
}

/// Opens the right reader for a picked path and imports it into a new archive.
pub fn import_paths(ar: &Archive, paths: &[PathBuf], progress: &dyn Fn(&ImportProgress)) -> R<ImportReport> {
    if paths.is_empty() {
        return Err("Nothing to import".into());
    }
    let name = if paths.len() == 1 {
        paths[0].file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Import".into())
    } else {
        format!("Import {}", chrono::Local::now().format("%Y-%m-%d %H:%M"))
    };
    let source = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("; ");
    let id = ar.create(&name, &source)?;
    let mut im = Importer::new(ar, id, &name, progress);
    for p in paths {
        im.import_path(p, "");
        if im.cancelled() {
            break;
        }
    }
    let rep = im.finish();
    if rep.added == 0 && rep.skipped == 0 {
        let _ = ar.delete(id);
        let why = rep.errors.first().cloned().unwrap_or_else(|| "no email messages were found".into());
        return Err(if rep.cancelled { "Import cancelled".into() } else { format!("Nothing was imported – {why}") });
    }
    Ok(rep)
}

// ============================================================ building RFC 5322 messages

#[derive(Default, Debug)]
pub struct OutMsg {
    pub from: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub date: i64,
    pub message_id: String,
    pub in_reply_to: String,
    pub references: String,
    pub text: String,
    pub html: String,
    pub attachments: Vec<OutAtt>,
}

#[derive(Default, Debug)]
pub struct OutAtt {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
    pub content_id: String,
}

fn enc_word(s: &str) -> String {
    use base64::Engine;
    let s = s.replace(['\r', '\n'], " ");
    if s.is_ascii() { s } else { format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(s.as_bytes())) }
}

/// "Name <a@b>" with the name encoded when needed; bare names (Exchange) are kept as display names.
fn enc_addr(name: &str, addr: &str) -> String {
    let name = name.trim().trim_matches('\'').trim_matches('"');
    let addr = addr.trim();
    match (name.is_empty(), addr.contains('@')) {
        (_, false) if name.is_empty() => String::new(),
        (true, true) => addr.to_string(),
        (false, true) if name.eq_ignore_ascii_case(addr) => addr.to_string(),
        (false, true) => {
            let n = if name.is_ascii() { format!("\"{}\"", name.replace('"', "'")) } else { enc_word(name) };
            format!("{n} <{addr}>")
        }
        (false, false) => {
            // no SMTP address (e.g. an Exchange-only recipient): keep the name readable
            let n = if name.is_ascii() { format!("\"{}\"", name.replace('"', "'")) } else { enc_word(name) };
            format!("{n} <>")
        }
        _ => String::new(),
    }
}

fn b64_lines(data: &[u8]) -> String {
    use base64::Engine;
    let s = base64::engine::general_purpose::STANDARD.encode(data);
    let mut out = String::with_capacity(s.len() + s.len() / 76 * 2 + 2);
    for chunk in s.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push_str("\r\n");
    }
    out
}

pub fn build_eml(m: &OutMsg) -> Vec<u8> {
    let mut h = String::new();
    let date = chrono::DateTime::from_timestamp(m.date.max(0), 0).unwrap_or_default();
    if !m.from.is_empty() {
        h.push_str(&format!("From: {}\r\n", m.from));
    }
    if !m.to.is_empty() {
        h.push_str(&format!("To: {}\r\n", m.to));
    }
    if !m.cc.is_empty() {
        h.push_str(&format!("Cc: {}\r\n", m.cc));
    }
    h.push_str(&format!("Subject: {}\r\n", enc_word(&m.subject)));
    h.push_str(&format!("Date: {}\r\n", date.to_rfc2822()));
    if !m.message_id.is_empty() {
        let id = m.message_id.trim();
        h.push_str(&format!("Message-ID: {}\r\n", if id.starts_with('<') { id.to_string() } else { format!("<{id}>") }));
    }
    if !m.in_reply_to.is_empty() {
        h.push_str(&format!("In-Reply-To: {}\r\n", m.in_reply_to.trim()));
    }
    if !m.references.is_empty() {
        h.push_str(&format!("References: {}\r\n", m.references.split_whitespace().collect::<Vec<_>>().join(" ")));
    }
    h.push_str("MIME-Version: 1.0\r\nX-Imported-By: Kite Mail\r\n");

    let text_part = |ct: &str, body: &str| format!("Content-Type: {ct}; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}", b64_lines(body.as_bytes()));
    let body = match (m.text.trim().is_empty(), m.html.trim().is_empty()) {
        (_, true) => text_part("text/plain", &m.text),
        (true, false) => text_part("text/html", &m.html),
        (false, false) => {
            let b = format!("kite-alt-{:016x}", rand::random::<u64>());
            format!(
                "Content-Type: multipart/alternative; boundary=\"{b}\"\r\n\r\n--{b}\r\n{}\r\n--{b}\r\n{}\r\n--{b}--\r\n",
                text_part("text/plain", &m.text),
                text_part("text/html", &m.html)
            )
        }
    };
    let mut out = h.into_bytes();
    if m.attachments.is_empty() {
        out.extend_from_slice(body.as_bytes());
        return out;
    }
    let b = format!("kite-mix-{:016x}", rand::random::<u64>());
    out.extend_from_slice(format!("Content-Type: multipart/mixed; boundary=\"{b}\"\r\n\r\n--{b}\r\n{body}\r\n").as_bytes());
    for a in &m.attachments {
        let fname = enc_word(&a.name).replace('"', "'");
        let mime = if a.mime.contains('/') { a.mime.as_str() } else { crate::smtp::guess_mime(&a.name) };
        let mut part = format!("--{b}\r\nContent-Type: {mime}; name=\"{fname}\"\r\n");
        if mime.eq_ignore_ascii_case("message/rfc822") {
            part.push_str(&format!("Content-Disposition: attachment; filename=\"{fname}\"\r\n\r\n"));
            out.extend_from_slice(part.as_bytes());
            out.extend_from_slice(&a.data);
            out.extend_from_slice(b"\r\n");
            continue;
        }
        let disp = if a.content_id.is_empty() { "attachment" } else { "inline" };
        part.push_str(&format!("Content-Disposition: {disp}; filename=\"{fname}\"\r\nContent-Transfer-Encoding: base64\r\n"));
        if !a.content_id.is_empty() {
            part.push_str(&format!("Content-ID: <{}>\r\n", a.content_id.trim_matches(['<', '>'])));
        }
        part.push_str("\r\n");
        out.extend_from_slice(part.as_bytes());
        out.extend_from_slice(b64_lines(&a.data).as_bytes());
    }
    out.extend_from_slice(format!("--{b}--\r\n").as_bytes());
    out
}

/// Windows FILETIME (100 ns since 1601) -> unix seconds.
fn filetime(ft: i64) -> i64 {
    if ft <= 0 { 0 } else { ft / 10_000_000 - 11_644_473_600 }
}

/// Outlook stores "RE: " etc. as a prefix length in the first two characters of some subjects.
fn clean_subject(s: &str) -> String {
    let mut c = s.chars();
    if s.starts_with('\u{1}') {
        c.next();
        c.next();
        return c.as_str().to_string();
    }
    s.to_string()
}

/// A property source (an Outlook .msg storage or a PST message / attachment).
trait Props {
    fn str(&self, id: u16) -> Option<String>;
    fn bin(&self, id: u16) -> Option<Vec<u8>>;
    fn int(&self, id: u16) -> Option<i32>;
    fn time(&self, id: u16) -> Option<i64>;
}

struct Recip {
    kind: i32,
    name: String,
    addr: String,
}

fn smtp_of(p: &dyn Props, ids: &[u16]) -> String {
    ids.iter().filter_map(|&id| p.str(id)).find(|a| a.contains('@')).unwrap_or_default()
}

/// Common MAPI -> RFC 5322 mapping for MSG and PST messages.
fn mapi_to_out(p: &dyn Props, recips: Vec<Recip>, attachments: Vec<OutAtt>) -> OutMsg {
    let mut m = OutMsg { attachments, ..Default::default() };
    // Original internet headers (present for mail that arrived over SMTP) are the best source.
    let headers = p.str(0x007D).unwrap_or_default();
    let header_block = format!("{}\r\n\r\n", headers.trim_end());
    let parsed = if headers.trim().is_empty() { None } else { MessageParser::default().parse(header_block.as_bytes()) };
    if let Some(hm) = parsed.as_ref() {
        m.from = mime::format_address_list(hm.from());
        m.to = mime::format_address_list(hm.to());
        m.cc = mime::format_address_list(hm.cc());
        m.date = hm.date().map(|d| d.to_timestamp()).unwrap_or(0);
        m.message_id = hm.message_id().map(|s| format!("<{s}>")).unwrap_or_default();
        m.in_reply_to = hm.in_reply_to().as_text_list().map(|l| l.iter().map(|s| format!("<{s}>")).collect::<Vec<_>>().join(" ")).unwrap_or_default();
        m.references = hm.references().as_text_list().map(|l| l.iter().map(|s| format!("<{s}>")).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    }
    if m.from.is_empty() {
        let name = p.str(0x0C1A).or_else(|| p.str(0x0042)).unwrap_or_default();
        let addr = smtp_of(p, &[0x5D01, 0x0C1F, 0x5D02, 0x0065]);
        m.from = enc_addr(&name, &addr);
    }
    if m.to.is_empty() && m.cc.is_empty() {
        let list = |kind: i32| recips.iter().filter(|r| r.kind == kind).map(|r| enc_addr(&r.name, &r.addr)).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
        m.to = list(1);
        m.cc = list(2);
        if m.to.is_empty() {
            m.to = p.str(0x0E04).map(|names| names.split(';').map(|n| enc_addr(n, "")).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        }
    }
    if m.date == 0 {
        m.date = p.time(0x0E06).or_else(|| p.time(0x0039)).or_else(|| p.time(0x3007)).map(filetime).unwrap_or(0);
    }
    if m.message_id.is_empty() {
        m.message_id = p.str(0x1035).unwrap_or_default();
    }
    if m.in_reply_to.is_empty() {
        m.in_reply_to = p.str(0x1042).unwrap_or_default();
    }
    if m.references.is_empty() {
        m.references = p.str(0x1039).unwrap_or_default();
    }
    m.subject = clean_subject(&p.str(0x0037).unwrap_or_default());
    m.text = p.str(0x1000).unwrap_or_default();
    m.html = p.bin(0x1013).map(|b| String::from_utf8_lossy(&b).into_owned()).or_else(|| p.str(0x1013)).unwrap_or_default();
    m
}

// ------------------------------------------------------------ Outlook .msg (Compound File Binary)

struct MsgStorage<'a, F: Read + std::io::Seek> {
    cf: &'a std::cell::RefCell<cfb::CompoundFile<F>>,
    base: String,
    /// size of the header in __properties_version1.0 (32 top message, 24 embedded, 8 recip/attach)
    header: usize,
}

impl<F: Read + std::io::Seek> MsgStorage<'_, F> {
    fn stream(&self, name: &str) -> Option<Vec<u8>> {
        let mut cf = self.cf.borrow_mut();
        let mut s = cf.open_stream(format!("{}/{name}", self.base)).ok()?;
        let mut b = Vec::new();
        s.read_to_end(&mut b).ok()?;
        Some(b)
    }

    fn fixed(&self, id: u16, ptype: u16) -> Option<[u8; 8]> {
        let b = self.stream("__properties_version1.0")?;
        let mut i = self.header;
        while i + 16 <= b.len() {
            let t = u16::from_le_bytes([b[i], b[i + 1]]);
            let pid = u16::from_le_bytes([b[i + 2], b[i + 3]]);
            if pid == id && t == ptype {
                let mut v = [0u8; 8];
                v.copy_from_slice(&b[i + 8..i + 16]);
                return Some(v);
            }
            i += 16;
        }
        None
    }
}

impl<F: Read + std::io::Seek> Props for MsgStorage<'_, F> {
    fn str(&self, id: u16) -> Option<String> {
        if let Some(b) = self.stream(&format!("__substg1.0_{id:04X}001F")) {
            let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            return Some(String::from_utf16_lossy(&u).trim_end_matches('\0').to_string());
        }
        self.stream(&format!("__substg1.0_{id:04X}001E")).map(|b| String::from_utf8_lossy(&b).trim_end_matches('\0').to_string())
    }
    fn bin(&self, id: u16) -> Option<Vec<u8>> {
        self.stream(&format!("__substg1.0_{id:04X}0102"))
    }
    fn int(&self, id: u16) -> Option<i32> {
        self.fixed(id, 0x0003).map(|v| i32::from_le_bytes([v[0], v[1], v[2], v[3]]))
    }
    fn time(&self, id: u16) -> Option<i64> {
        self.fixed(id, 0x0040).map(i64::from_le_bytes)
    }
}

/// Converts an Outlook .msg file to a standard email.
pub fn msg_to_eml(bytes: &[u8]) -> R<Vec<u8>> {
    let cf = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).map_err(|_| "This is not an Outlook .msg file".to_string())?;
    let cell = std::cell::RefCell::new(cf);
    let out = msg_storage_to_out(&cell, "", 32)?;
    Ok(build_eml(&out))
}

fn msg_storage_to_out<F: Read + std::io::Seek>(cell: &std::cell::RefCell<cfb::CompoundFile<F>>, base: &str, header: usize) -> R<OutMsg> {
    let top = MsgStorage { cf: cell, base: base.to_string(), header };
    let children: Vec<String> = {
        let cf = cell.borrow();
        let dir = if base.is_empty() { "/".to_string() } else { base.to_string() };
        cf.read_storage(&dir).map_err(e)?.filter(|en| en.is_storage()).map(|en| en.name().to_string()).collect()
    };
    let mut recips = Vec::new();
    let mut atts = Vec::new();
    for name in &children {
        let path = format!("{base}/{name}");
        if name.starts_with("__recip_version1.0_") {
            let r = MsgStorage { cf: cell, base: path, header: 8 };
            recips.push(Recip {
                kind: r.int(0x0C15).unwrap_or(1),
                name: r.str(0x3001).unwrap_or_default(),
                addr: smtp_of(&r, &[0x39FE, 0x3003]),
            });
        } else if name.starts_with("__attach_version1.0_") {
            let a = MsgStorage { cf: cell, base: path.clone(), header: 8 };
            let fname = a.str(0x3707).or_else(|| a.str(0x3704)).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "attachment".into());
            let embedded = format!("{path}/__substg1.0_3701000D");
            if cell.borrow().is_storage(&embedded) {
                if let Ok(inner) = msg_storage_to_out(cell, &embedded, 24) {
                    let title = if inner.subject.is_empty() { "message".to_string() } else { inner.subject.clone() };
                    atts.push(OutAtt { name: format!("{}.eml", title.replace(['/', '\\', ':'], "-")), mime: "message/rfc822".into(), data: build_eml(&inner), content_id: String::new() });
                }
                continue;
            }
            if let Some(data) = a.bin(0x3701) {
                atts.push(OutAtt { name: fname, mime: a.str(0x370E).unwrap_or_default(), data, content_id: a.str(0x3712).unwrap_or_default() });
            }
        }
    }
    Ok(mapi_to_out(&top, recips, atts))
}

// ------------------------------------------------------------ Outlook .pst

mod pst {
    use super::{clean_subject, Importer, OutAtt, Props, Recip, R};
    use outlook_pst::ltp::prop_context::PropertyValue;
    use outlook_pst::messaging::{attachment::Attachment, message::Message, store::Store};
    use outlook_pst::ndb::node_id::NodeId;
    use std::rc::Rc;

    fn value_str(v: Option<&PropertyValue>) -> Option<String> {
        match v {
            Some(PropertyValue::Unicode(u)) => Some(u.to_string()),
            Some(PropertyValue::String8(a)) => Some(a.to_string()),
            _ => None,
        }
    }

    struct P<'a>(&'a dyn Fn(u16) -> Option<PropertyValue>);

    impl Props for P<'_> {
        fn str(&self, id: u16) -> Option<String> {
            value_str((self.0)(id).as_ref())
        }
        fn bin(&self, id: u16) -> Option<Vec<u8>> {
            match (self.0)(id) {
                Some(PropertyValue::Binary(b)) => Some(b.buffer().to_vec()),
                _ => None,
            }
        }
        fn int(&self, id: u16) -> Option<i32> {
            match (self.0)(id) {
                Some(PropertyValue::Integer32(i)) => Some(i),
                _ => None,
            }
        }
        fn time(&self, id: u16) -> Option<i64> {
            match (self.0)(id) {
                Some(PropertyValue::Time(t)) => Some(t),
                _ => None,
            }
        }
    }

    /// Mail-like Outlook items (skips contacts, calendar, tasks, notes, journal).
    fn is_mail(class: &str) -> bool {
        let c = class.to_ascii_uppercase();
        c.is_empty() || c.starts_with("IPM.NOTE") || c.starts_with("IPM.SCHEDULE") || c.starts_with("REPORT.") || c.starts_with("IPM.POST")
    }

    macro_rules! importer {
        ($fname:ident, $file:ty, $store:ty, $msg:ty, $att:ty) => {
            pub fn $fname(im: &mut Importer, path: &std::path::Path, prefix: &str) -> R<()> {
                let pst = Rc::new(<$file>::open(path).map_err(|e| e.to_string())?);
                let store: Rc<$store> = <$store>::read(pst).map_err(|e| e.to_string())?;
                let root = store.properties().ipm_sub_tree_entry_id().map_err(|e| e.to_string())?;
                let mut stack = vec![(store.open_folder(&root).map_err(|e| e.to_string())?, prefix.to_string())];
                while let Some((folder, fpath)) = stack.pop() {
                    if let Some(h) = folder.hierarchy_table() {
                        let mut subs = Vec::new();
                        for row in h.rows_matrix() {
                            let Ok(eid) = store.properties().make_entry_id(NodeId::from(u32::from(row.id()))) else { continue };
                            let Ok(f) = store.open_folder(&eid) else { continue };
                            let name = value_str(f.properties().get(0x3001)).unwrap_or_else(|| "Folder".into());
                            subs.push((f, super::join_folder(&fpath, &name)));
                        }
                        subs.reverse(); // pop() order = original order
                        stack.extend(subs);
                    }
                    let Some(ct) = folder.contents_table() else { continue };
                    let folder_name = if fpath.is_empty() { "Imported".to_string() } else { fpath.clone() };
                    for row in ct.rows_matrix() {
                        if im.cancelled() {
                            return Ok(());
                        }
                        let what = format!("{} / {}", im.file, folder_name);
                        let eid = match store.properties().make_entry_id(NodeId::from(u32::from(row.id()))) {
                            Ok(e) => e,
                            Err(err) => { im.fail(&what, &err.to_string()); continue }
                        };
                        let msg = match <$msg>::read(store.clone(), &eid, None) {
                            Ok(m) => m,
                            Err(err) => { im.fail(&what, &err.to_string()); continue }
                        };
                        let props = msg.properties();
                        let class = value_str(props.get(0x001A)).unwrap_or_default();
                        if !is_mail(&class) {
                            continue;
                        }
                        // recipients
                        let mut recips = Vec::new();
                        if let Some(rt) = msg.recipient_table() {
                            let ctx = rt.context();
                            for r in rt.rows_matrix() {
                                let Ok(cols) = r.columns(ctx) else { continue };
                                let mut rec = Recip { kind: 1, name: String::new(), addr: String::new() };
                                let mut email = String::new();
                                for (col, val) in ctx.columns().iter().zip(cols) {
                                    let Some(val) = val else { continue };
                                    let Ok(v) = rt.read_column(&val, col.prop_type()) else { continue };
                                    match (col.prop_id(), v) {
                                        (0x0C15, PropertyValue::Integer32(k)) => rec.kind = k,
                                        (0x3001, v) => rec.name = value_str(Some(&v)).unwrap_or_default(),
                                        (0x39FE, v) => rec.addr = value_str(Some(&v)).unwrap_or_default(),
                                        (0x3003, v) => email = value_str(Some(&v)).unwrap_or_default(),
                                        _ => {}
                                    }
                                }
                                if !rec.addr.contains('@') && email.contains('@') {
                                    rec.addr = email;
                                }
                                recips.push(rec);
                            }
                        }
                        // attachments (embedded Outlook items are skipped)
                        let mut atts = Vec::new();
                        if let Some(at) = msg.attachment_table() {
                            for r in at.rows_matrix() {
                                let Ok(a) = <$att>::read(msg.clone(), NodeId::from(u32::from(r.id())), None) else { continue };
                                let ap = a.properties();
                                let data = match ap.get(0x3701) { Some(PropertyValue::Binary(b)) => b.buffer().to_vec(), _ => continue };
                                let name = value_str(ap.get(0x3707)).or_else(|| value_str(ap.get(0x3704))).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "attachment".into());
                                atts.push(OutAtt { name, mime: value_str(ap.get(0x370E)).unwrap_or_default(), data, content_id: value_str(ap.get(0x3712)).unwrap_or_default() });
                            }
                        }
                        let get = |id: u16| props.get(id).cloned();
                        let mut out = super::mapi_to_out(&P(&get), recips, atts);
                        out.subject = clean_subject(&out.subject);
                        let raw = super::build_eml(&out);
                        im.add(&folder_name, &raw);
                    }
                }
                Ok(())
            }
        };
    }

    importer!(import_unicode, outlook_pst::UnicodePstFile, outlook_pst::messaging::store::UnicodeStore, outlook_pst::messaging::message::UnicodeMessage, outlook_pst::messaging::attachment::UnicodeAttachment);
    importer!(import_ansi, outlook_pst::AnsiPstFile, outlook_pst::messaging::store::AnsiStore, outlook_pst::messaging::message::AnsiMessage, outlook_pst::messaging::attachment::AnsiAttachment);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet(_: &ImportProgress) {}

    fn eml(subject: &str, mid: &str, body: &str) -> Vec<u8> {
        format!("From: Sara Khan <sara@example.ae>\r\nTo: akhil@example.ae\r\nSubject: {subject}\r\nDate: Mon, 3 Nov 2025 10:00:00 +0400\r\nMessage-ID: <{mid}>\r\n\r\n{body}\r\n").into_bytes()
    }

    #[test]
    fn eml_mbox_maildir_zip_search_and_dedupe() {
        let dir = std::env::temp_dir().join(format!("kite-arch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // a folder with an .eml, an mbox without extension, a maildir with a Maildir++ subfolder
        std::fs::create_dir_all(dir.join("Clients/Dubai")).unwrap();
        std::fs::write(dir.join("Clients/Dubai/visa.eml"), eml("Golden visa renewal", "a1@x", "Please renew the golden visa for Ahmed")).unwrap();
        let mut mbox = Vec::new();
        for (i, s) in ["Invoice March", "Invoice April"].iter().enumerate() {
            mbox.extend_from_slice(b"From sara@example.ae Mon Nov  3 10:00:00 2025\n");
            mbox.extend_from_slice(&eml(s, &format!("m{i}@x"), "amount due\n>From the accounts team"));
            mbox.extend_from_slice(b"\n");
        }
        std::fs::write(dir.join("Sent"), &mbox).unwrap();
        for sub in ["md/cur", "md/new", "md/tmp", "md/.Projects.TLR/cur", "md/.Projects.TLR/new"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join("md/cur/1700000000.M1P1.host"), eml("Court hearing moved", "c1@x", "Thursday 10am")).unwrap();
        std::fs::write(dir.join("md/.Projects.TLR/new/2"), eml("TLR website launch", "t1@x", "launch checklist")).unwrap();
        std::fs::write(dir.join("notes.docx"), b"not mail").unwrap();

        let ar = Archive::memory();
        let rep = import_paths(&ar, std::slice::from_ref(&dir), &quiet).unwrap();
        assert_eq!((rep.added, rep.failed), (5, 0), "{:?}", rep.errors);
        let info = &ar.list().unwrap()[0];
        let folders: Vec<&str> = info.folders.iter().map(|f| f.name.as_str()).collect();
        assert!(folders.contains(&"Clients/Dubai") && folders.contains(&"Sent") && folders.contains(&"md") && folders.contains(&"md/Projects/TLR"), "{folders:?}");

        // full-text: words, prefixes, people
        assert_eq!(ar.search(None, "golden ahm", 10).unwrap().len(), 1);
        assert_eq!(ar.search(None, "invoice", 10).unwrap().len(), 2);
        assert_eq!(ar.search(None, "sara@example.ae hearing", 10).unwrap().len(), 1);
        assert!(ar.search(None, "\"; DROP TABLE amsg; --", 10).unwrap().is_empty());
        // mboxrd unquoting and round trip of the stored message
        let inv = &ar.search(None, "april", 10).unwrap()[0];
        let (_, raw) = ar.get(inv.id).unwrap();
        assert!(String::from_utf8_lossy(&raw).contains("\nFrom the accounts team") || String::from_utf8_lossy(&raw).contains("From the accounts team"));

        // importing the same files again: everything is a duplicate
        let mut im = Importer::new(&ar, info.id, "x", &quiet);
        im.import_path(&dir, "");
        let rep2 = im.finish();
        assert_eq!((rep2.added, rep2.skipped), (0, 5));

        // Kite Mail backup zip
        let zpath = dir.join("backup.zip");
        {
            let f = std::fs::File::create(&zpath).unwrap();
            let mut z = zip::ZipWriter::new(f);
            z.start_file("INBOX.mbox", zip::write::SimpleFileOptions::default()).unwrap();
            let mut one = b"From x Mon Nov  3 10:00:00 2025\n".to_vec();
            one.extend_from_slice(crate::mail::FLAGS_HEADER.as_bytes());
            one.extend_from_slice(b"\\Seen\n");
            one.extend_from_slice(&eml("From the backup", "z1@x", "zipped"));
            z.write_all(&one).unwrap();
            z.finish().unwrap();
        }
        let rep3 = import_paths(&ar, &[zpath], &quiet).unwrap();
        assert_eq!(rep3.added, 1);
        let m = &ar.messages(rep3.archive_id, Some("INBOX"), 0, 10).unwrap()[0];
        let (_, raw) = ar.get(m.id).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("X-KiteMail-Flags"));

        // nothing importable -> clear error, no empty archive left behind
        let n = ar.list().unwrap().len();
        assert!(import_paths(&ar, &[dir.join("notes.docx")], &quiet).is_err());
        assert_eq!(ar.list().unwrap().len(), n);

        ar.delete(info.id).unwrap();
        assert!(ar.search(Some(info.id), "invoice", 10).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn built_messages_parse_back() {
        let out = OutMsg {
            from: enc_addr("Ahmed Al Mansoori", "ahmed@example.ae"),
            to: [enc_addr("فراس", "firas@example.ae"), enc_addr("Exchange User", "")].join(", "),
            subject: "Contract – Été 2026".into(),
            date: 1_700_000_000,
            message_id: "abc@x".into(),
            text: "Hello\nworld".into(),
            html: "<p>Hello <b>world</b></p>".into(),
            attachments: vec![OutAtt { name: "Résumé.pdf".into(), mime: "application/pdf".into(), data: b"%PDF-1.4".to_vec(), content_id: String::new() }],
            ..Default::default()
        };
        let raw = build_eml(&out);
        let m = MessageParser::default().parse(&raw).unwrap();
        assert_eq!(m.subject(), Some("Contract – Été 2026"));
        assert_eq!(m.from().unwrap().first().unwrap().address(), Some("ahmed@example.ae"));
        assert_eq!(m.to().unwrap().first().unwrap().name(), Some("فراس"));
        assert_eq!(m.message_id(), Some("abc@x"));
        assert_eq!(m.date().unwrap().to_timestamp(), 1_700_000_000);
        assert!(m.body_html(0).unwrap().contains("<b>world</b>"));
        let a = m.attachments().next().unwrap();
        assert_eq!(a.attachment_name(), Some("Résumé.pdf"));
        assert_eq!(a.contents(), b"%PDF-1.4");
        assert_eq!(clean_subject("\u{1}\u{1}Hello"), "Hello");
        assert_eq!(filetime(132_000_000_000_000_000), 1_555_526_400);
    }

    /// Real Outlook files (MSG samples from msg-extractor, the EDRM Enron PST). Skipped when the
    /// sample files aren't present – see README "Development".
    #[test]
    fn outlook_samples() {
        let msg = Path::new("/tmp/msgsamples/me/example-msg-files/unicode.msg");
        let pst = Path::new("/tmp/pstsamples/pe/src/__tests__/testdata/enron.pst");
        if !msg.exists() || !pst.exists() {
            eprintln!("skipped: Outlook sample files not present");
            return;
        }
        let raw = msg_to_eml(&std::fs::read(msg).unwrap()).unwrap();
        let m = MessageParser::default().parse(&raw).unwrap();
        assert!(!m.subject().unwrap_or("").is_empty());
        assert!(m.from().is_some());
        let emb = msg_to_eml(&std::fs::read("/tmp/msgsamples/me/example-msg-files/test-embedded.msg").unwrap()).unwrap();
        let em = MessageParser::default().parse(&emb).unwrap();
        assert!(em.attachments().count() >= 1);

        let ar = Archive::memory();
        let rep = import_paths(&ar, &[pst.to_path_buf()], &quiet).unwrap();
        eprintln!("enron.pst: {} added, {} duplicates, {} failed {:?}", rep.added, rep.skipped, rep.failed, rep.errors.first());
        assert!(rep.added >= 60, "{rep:?}");
        assert!(rep.failed <= rep.added / 10, "{rep:?}");
        let hits = ar.search(None, "Tahoe", 10).unwrap();
        assert!(!hits.is_empty());
        let trip = ar.search(None, "TRIP INFO", 5).unwrap();
        let (_, raw) = ar.get(trip[0].id).unwrap();
        let tm = MessageParser::default().parse(&raw).unwrap();
        let names: Vec<_> = tm.attachments().filter_map(|a| a.attachment_name().map(String::from)).collect();
        assert!(names.iter().any(|n| n.ends_with(".doc")) && names.iter().any(|n| n.ends_with(".xls")), "{names:?}");
        assert!(!trip[0].subject.starts_with('\u{1}'));
    }
}
