use crate::models::{Account, AddressSuggestion, Contact, Folder, MsgSummary, Todo};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
}

type R<T> = Result<T, String>;

fn e(err: rusqlite::Error) -> String {
    format!("Database error: {err}")
}

pub fn data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    let dir = base.join("KiteMail");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

const SCHEMA: &str = r#"
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS accounts(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL, email TEXT NOT NULL, kind TEXT NOT NULL,
  imap_host TEXT NOT NULL, imap_port INTEGER NOT NULL,
  smtp_host TEXT NOT NULL, smtp_port INTEGER NOT NULL, smtp_security TEXT NOT NULL,
  username TEXT NOT NULL, tenant TEXT NOT NULL DEFAULT '',
  client_id TEXT NOT NULL DEFAULT '', client_secret TEXT NOT NULL DEFAULT '',
  save_sent INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS folders(
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  name TEXT NOT NULL, display TEXT NOT NULL, special TEXT NOT NULL DEFAULT '',
  noselect INTEGER NOT NULL DEFAULT 0, depth INTEGER NOT NULL DEFAULT 0,
  sort INTEGER NOT NULL DEFAULT 0, uidvalidity INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(account_id, name)
);
CREATE TABLE IF NOT EXISTS messages(
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  folder TEXT NOT NULL, uid INTEGER NOT NULL,
  subject TEXT NOT NULL DEFAULT '', from_name TEXT NOT NULL DEFAULT '',
  from_addr TEXT NOT NULL DEFAULT '', to_addrs TEXT NOT NULL DEFAULT '',
  date INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0,
  seen INTEGER NOT NULL DEFAULT 0, flagged INTEGER NOT NULL DEFAULT 0,
  answered INTEGER NOT NULL DEFAULT 0, has_attach INTEGER NOT NULL DEFAULT 0,
  raw BLOB, body_text TEXT,
  PRIMARY KEY(account_id, folder, uid)
);
CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(account_id, folder, date DESC);
CREATE TABLE IF NOT EXISTS secrets(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS contacts(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL DEFAULT '', email TEXT NOT NULL DEFAULT '', phone TEXT NOT NULL DEFAULT '',
  company TEXT NOT NULL DEFAULT '', notes TEXT NOT NULL DEFAULT '', favorite INTEGER NOT NULL DEFAULT 0,
  updated INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_contacts_email ON contacts(email);
CREATE TABLE IF NOT EXISTS todos(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL, notes TEXT NOT NULL DEFAULT '', due INTEGER, done INTEGER NOT NULL DEFAULT 0,
  priority INTEGER NOT NULL DEFAULT 0, account_id INTEGER, folder TEXT, uid INTEGER,
  created INTEGER NOT NULL DEFAULT 0, notified INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS drafts(
  id INTEGER PRIMARY KEY AUTOINCREMENT, json TEXT NOT NULL, pending INTEGER NOT NULL DEFAULT 1, updated INTEGER NOT NULL
);
"#;

/// Columns added after v0.1 - applied to existing databases, errors ignored if already present.
const MIGRATIONS: &[&str] = &[
    "ALTER TABLE messages ADD COLUMN preview TEXT NOT NULL DEFAULT ''",
    // v0.3: CardDAV / CalDAV sync state
    "ALTER TABLE contacts ADD COLUMN uid TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE contacts ADD COLUMN href TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE contacts ADD COLUMN etag TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE contacts ADD COLUMN raw TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE contacts ADD COLUMN dirty INTEGER NOT NULL DEFAULT 1",
    "ALTER TABLE contacts ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE todos ADD COLUMN dav_uid TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE todos ADD COLUMN href TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE todos ADD COLUMN etag TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE todos ADD COLUMN raw TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE todos ADD COLUMN dirty INTEGER NOT NULL DEFAULT 1",
    "ALTER TABLE todos ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0",
];

/// Sync bookkeeping for one contact / todo row.
#[derive(Debug, Clone)]
pub struct SyncRow {
    pub id: i64,
    pub uid: String,
    pub href: String,
    pub etag: String,
    pub raw: String,
    pub dirty: bool,
    pub deleted: bool,
}

pub fn new_uid() -> String {
    use rand::Rng;
    let r: u128 = rand::thread_rng().gen();
    let h = format!("{r:032x}");
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

const SUMMARY_COLS: &str = "account_id, folder, uid, subject, from_name, from_addr, to_addrs, date, size, seen, flagged, answered, has_attach, preview";

fn row_to_summary(r: &Row) -> rusqlite::Result<MsgSummary> {
    Ok(MsgSummary {
        account_id: r.get(0)?,
        folder: r.get(1)?,
        uid: r.get(2)?,
        subject: r.get(3)?,
        from_name: r.get(4)?,
        from_addr: r.get(5)?,
        to: r.get(6)?,
        date: r.get(7)?,
        size: r.get(8)?,
        seen: r.get(9)?,
        flagged: r.get(10)?,
        answered: r.get(11)?,
        has_attach: r.get(12)?,
        preview: r.get(13)?,
    })
}

fn row_to_account(r: &Row) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get(0)?,
        name: r.get(1)?,
        email: r.get(2)?,
        kind: r.get(3)?,
        imap_host: r.get(4)?,
        imap_port: r.get(5)?,
        smtp_host: r.get(6)?,
        smtp_port: r.get(7)?,
        smtp_security: r.get(8)?,
        username: r.get(9)?,
        tenant: r.get(10)?,
        client_id: r.get(11)?,
        client_secret: r.get(12)?,
        save_sent: r.get(13)?,
    })
}

const ACCOUNT_COLS: &str = "id, name, email, kind, imap_host, imap_port, smtp_host, smtp_port, smtp_security, username, tenant, client_id, client_secret, save_sent";

impl Db {
    pub fn open(path: PathBuf) -> R<Db> {
        let conn = Connection::open(path).map_err(e)?;
        conn.execute_batch(SCHEMA).map_err(e)?;
        for m in MIGRATIONS {
            let _ = conn.execute(m, []);
        }
        let db = Db { conn: Mutex::new(conn) };
        db.backfill_previews();
        Ok(db)
    }

    /// Fills previews for bodies cached by older versions (runs once per message).
    fn backfill_previews(&self) {
        let rows: Vec<(i64, String, u32, Vec<u8>)> = {
            let c = self.c();
            let Ok(mut st) = c.prepare("SELECT account_id, folder, uid, raw FROM messages WHERE raw IS NOT NULL AND preview='' LIMIT 3000") else { return };
            let it = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)));
            match it { Ok(i) => i.flatten().collect(), Err(_) => return }
        };
        for (a, f, u, raw) in rows {
            let text = crate::mime::search_text(&raw);
            let p = crate::mime::preview(&text);
            let _ = self.c().execute(
                "UPDATE messages SET body_text=?4, preview=?5 WHERE account_id=?1 AND folder=?2 AND uid=?3",
                params![a, f, u, text, if p.is_empty() { " ".to_string() } else { p }],
            );
        }
    }

    #[cfg(test)]
    pub fn memory() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        for m in MIGRATIONS {
            let _ = conn.execute(m, []);
        }
        Db { conn: Mutex::new(conn) }
    }

    fn c(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    // ---------- accounts ----------
    /// Saves edited account details (Server Settings / identity). The address and type stay.
    pub fn update_account(&self, a: &Account) -> R<()> {
        self.c()
            .execute(
                "UPDATE accounts SET name=?2, imap_host=?3, imap_port=?4, smtp_host=?5, smtp_port=?6, smtp_security=?7, username=?8, save_sent=?9 WHERE id=?1",
                params![a.id, a.name, a.imap_host, a.imap_port, a.smtp_host, a.smtp_port, a.smtp_security, a.username, a.save_sent],
            )
            .map_err(e)?;
        Ok(())
    }

    pub fn insert_account(&self, a: &Account) -> R<i64> {
        let c = self.c();
        c.execute(
            "INSERT INTO accounts(name,email,kind,imap_host,imap_port,smtp_host,smtp_port,smtp_security,username,tenant,client_id,client_secret,save_sent)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![a.name, a.email, a.kind, a.imap_host, a.imap_port, a.smtp_host, a.smtp_port,
                    a.smtp_security, a.username, a.tenant, a.client_id, a.client_secret, a.save_sent],
        ).map_err(e)?;
        Ok(c.last_insert_rowid())
    }

    pub fn accounts(&self) -> R<Vec<Account>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT {ACCOUNT_COLS} FROM accounts ORDER BY id")).map_err(e)?;
        let rows = st.query_map([], row_to_account).map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    pub fn account(&self, id: i64) -> R<Account> {
        let c = self.c();
        c.query_row(&format!("SELECT {ACCOUNT_COLS} FROM accounts WHERE id=?1"), [id], row_to_account)
            .optional()
            .map_err(e)?
            .ok_or_else(|| "Account not found".to_string())
    }

    pub fn delete_account(&self, id: i64) -> R<()> {
        let c = self.c();
        c.execute("DELETE FROM messages WHERE account_id=?1", [id]).map_err(e)?;
        c.execute("DELETE FROM folders WHERE account_id=?1", [id]).map_err(e)?;
        c.execute("DELETE FROM drafts WHERE json_extract(json, '$.account_id')=?1", [id]).map_err(e)?;
        c.execute("DELETE FROM accounts WHERE id=?1", [id]).map_err(e)?;
        Ok(())
    }

    // ---------- folders ----------
    pub fn replace_folders(&self, account_id: i64, folders: &[(Folder, i64)]) -> R<()> {
        let mut c = self.c();
        let tx = c.transaction().map_err(e)?;
        let names: HashSet<&str> = folders.iter().map(|(f, _)| f.name.as_str()).collect();
        {
            let mut existing = tx.prepare("SELECT name FROM folders WHERE account_id=?1").map_err(e)?;
            let old: Vec<String> = existing
                .query_map([account_id], |r| r.get(0)).map_err(e)?
                .filter_map(Result::ok).collect();
            for n in old.iter().filter(|n| !names.contains(n.as_str())) {
                tx.execute("DELETE FROM folders WHERE account_id=?1 AND name=?2", params![account_id, n]).map_err(e)?;
                tx.execute("DELETE FROM messages WHERE account_id=?1 AND folder=?2", params![account_id, n]).map_err(e)?;
            }
        }
        for (f, sort) in folders {
            tx.execute(
                "INSERT INTO folders(account_id,name,display,special,noselect,depth,sort) VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(account_id,name) DO UPDATE SET display=?3, special=?4, noselect=?5, depth=?6, sort=?7",
                params![account_id, f.name, f.display, f.special, f.noselect, f.depth, sort],
            ).map_err(e)?;
        }
        tx.commit().map_err(e)
    }

    pub fn folders(&self, account_id: i64) -> R<Vec<Folder>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT f.account_id, f.name, f.display, f.special, f.noselect, f.depth,
                    (SELECT COUNT(*) FROM messages m WHERE m.account_id=f.account_id AND m.folder=f.name AND m.seen=0)
             FROM folders f WHERE f.account_id=?1 ORDER BY f.sort",
        ).map_err(e)?;
        let rows = st.query_map([account_id], |r| {
            Ok(Folder {
                account_id: r.get(0)?,
                name: r.get(1)?,
                display: r.get(2)?,
                special: r.get(3)?,
                noselect: r.get(4)?,
                depth: r.get(5)?,
                unread: r.get(6)?,
            })
        }).map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    pub fn folder_by_special(&self, account_id: i64, special: &str) -> R<Option<String>> {
        let c = self.c();
        c.query_row(
            "SELECT name FROM folders WHERE account_id=?1 AND special=?2 LIMIT 1",
            params![account_id, special],
            |r| r.get(0),
        ).optional().map_err(e)
    }

    pub fn uidvalidity(&self, account_id: i64, folder: &str) -> R<u32> {
        let c = self.c();
        Ok(c.query_row(
            "SELECT uidvalidity FROM folders WHERE account_id=?1 AND name=?2",
            params![account_id, folder],
            |r| r.get(0),
        ).optional().map_err(e)?.unwrap_or(0))
    }

    pub fn reset_folder(&self, account_id: i64, folder: &str, uidvalidity: u32) -> R<()> {
        let c = self.c();
        c.execute("DELETE FROM messages WHERE account_id=?1 AND folder=?2", params![account_id, folder]).map_err(e)?;
        c.execute("UPDATE folders SET uidvalidity=?3 WHERE account_id=?1 AND name=?2", params![account_id, folder, uidvalidity]).map_err(e)?;
        Ok(())
    }

    // ---------- messages ----------
    pub fn cached_uids(&self, account_id: i64, folder: &str) -> R<HashSet<u32>> {
        let c = self.c();
        let mut st = c.prepare("SELECT uid FROM messages WHERE account_id=?1 AND folder=?2").map_err(e)?;
        let rows = st.query_map(params![account_id, folder], |r| r.get::<_, u32>(0)).map_err(e)?;
        rows.collect::<rusqlite::Result<HashSet<_>>>().map_err(e)
    }

    pub fn uids_without_body(&self, account_id: i64, folder: &str, max_size: u32, limit: u32) -> R<Vec<u32>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT uid FROM messages WHERE account_id=?1 AND folder=?2 AND raw IS NULL AND size<=?3 ORDER BY date DESC LIMIT ?4",
        ).map_err(e)?;
        let rows = st.query_map(params![account_id, folder, max_size, limit], |r| r.get::<_, u32>(0)).map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    pub fn upsert_summaries(&self, list: &[MsgSummary]) -> R<()> {
        let mut c = self.c();
        let tx = c.transaction().map_err(e)?;
        for m in list {
            tx.execute(
                "INSERT INTO messages(account_id,folder,uid,subject,from_name,from_addr,to_addrs,date,size,seen,flagged,answered,has_attach)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
                 ON CONFLICT(account_id,folder,uid) DO UPDATE SET subject=?4, from_name=?5, from_addr=?6, to_addrs=?7,
                   date=?8, size=?9, seen=?10, flagged=?11, answered=?12, has_attach=?13",
                params![m.account_id, m.folder, m.uid, m.subject, m.from_name, m.from_addr, m.to, m.date,
                        m.size, m.seen, m.flagged, m.answered, m.has_attach],
            ).map_err(e)?;
        }
        tx.commit().map_err(e)
    }

    pub fn update_flags(&self, account_id: i64, folder: &str, flags: &[(u32, bool, bool, bool)]) -> R<()> {
        let mut c = self.c();
        let tx = c.transaction().map_err(e)?;
        for (uid, seen, flagged, answered) in flags {
            tx.execute(
                "UPDATE messages SET seen=?4, flagged=?5, answered=?6 WHERE account_id=?1 AND folder=?2 AND uid=?3",
                params![account_id, folder, uid, seen, flagged, answered],
            ).map_err(e)?;
        }
        tx.commit().map_err(e)
    }

    pub fn set_flag(&self, account_id: i64, folder: &str, uids: &[u32], column: &str, value: bool) -> R<()> {
        let col = match column {
            "seen" | "flagged" | "answered" => column,
            _ => return Err("bad flag".into()),
        };
        let mut c = self.c();
        let tx = c.transaction().map_err(e)?;
        for uid in uids {
            tx.execute(
                &format!("UPDATE messages SET {col}=?4 WHERE account_id=?1 AND folder=?2 AND uid=?3"),
                params![account_id, folder, uid, value],
            ).map_err(e)?;
        }
        tx.commit().map_err(e)
    }

    pub fn delete_messages(&self, account_id: i64, folder: &str, uids: &[u32]) -> R<()> {
        let mut c = self.c();
        let tx = c.transaction().map_err(e)?;
        for uid in uids {
            tx.execute("DELETE FROM messages WHERE account_id=?1 AND folder=?2 AND uid=?3", params![account_id, folder, uid]).map_err(e)?;
        }
        tx.commit().map_err(e)
    }

    pub fn list_messages(&self, account_id: i64, folder: &str, limit: u32) -> R<Vec<MsgSummary>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT {SUMMARY_COLS} FROM messages WHERE account_id=?1 AND folder=?2 ORDER BY date DESC, uid DESC LIMIT ?3"
        )).map_err(e)?;
        let rows = st.query_map(params![account_id, folder, limit], row_to_summary).map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    pub fn summary(&self, account_id: i64, folder: &str, uid: u32) -> R<Option<MsgSummary>> {
        let c = self.c();
        c.query_row(
            &format!("SELECT {SUMMARY_COLS} FROM messages WHERE account_id=?1 AND folder=?2 AND uid=?3"),
            params![account_id, folder, uid],
            row_to_summary,
        ).optional().map_err(e)
    }

    pub fn raw(&self, account_id: i64, folder: &str, uid: u32) -> R<Option<Vec<u8>>> {
        let c = self.c();
        Ok(c.query_row(
            "SELECT raw FROM messages WHERE account_id=?1 AND folder=?2 AND uid=?3",
            params![account_id, folder, uid],
            |r| r.get::<_, Option<Vec<u8>>>(0),
        ).optional().map_err(e)?.flatten())
    }

    pub fn store_raw(&self, account_id: i64, folder: &str, uid: u32, raw: &[u8], text: &str) -> R<()> {
        let c = self.c();
        c.execute(
            "UPDATE messages SET raw=?4, body_text=?5, preview=?6 WHERE account_id=?1 AND folder=?2 AND uid=?3",
            params![account_id, folder, uid, raw, text, crate::mime::preview(text)],
        ).map_err(e)?;
        Ok(())
    }

    pub fn max_uid(&self, account_id: i64, folder: &str) -> R<u32> {
        self.c().query_row(
            "SELECT COALESCE(MAX(uid),0) FROM messages WHERE account_id=?1 AND folder=?2",
            params![account_id, folder], |r| r.get(0),
        ).map_err(e)
    }

    pub fn new_since(&self, account_id: i64, folder: &str, after_uid: u32) -> R<Vec<MsgSummary>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT {SUMMARY_COLS} FROM messages WHERE account_id=?1 AND folder=?2 AND uid>?3 AND seen=0 ORDER BY uid DESC"
        )).map_err(e)?;
        let rows = st.query_map(params![account_id, folder, after_uid], row_to_summary).map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    // ---------- settings ----------
    pub fn get_setting(&self, key: &str) -> Option<String> {
        self.c().query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).ok()
    }
    pub fn set_setting(&self, key: &str, value: &str) -> R<()> {
        self.c().execute(
            "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2",
            params![key, value],
        ).map_err(e)?;
        Ok(())
    }

    // ---------- contacts ----------
    pub fn contacts(&self, query: &str) -> R<Vec<Contact>> {
        let c = self.c();
        let like = format!("%{}%", query.trim());
        let mut st = c.prepare(
            "SELECT id,name,email,phone,company,notes,favorite FROM contacts
             WHERE deleted=0 AND (?1='%%' OR name LIKE ?1 OR email LIKE ?1 OR phone LIKE ?1 OR company LIKE ?1)
             ORDER BY favorite DESC, name COLLATE NOCASE, email",
        ).map_err(e)?;
        let rows = st.query_map([like], |r| Ok(Contact {
            id: r.get(0)?, name: r.get(1)?, email: r.get(2)?, phone: r.get(3)?,
            company: r.get(4)?, notes: r.get(5)?, favorite: r.get(6)?,
        })).map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    pub fn save_contact(&self, ct: &Contact) -> R<i64> {
        let c = self.c();
        if ct.id > 0 {
            c.execute(
                "UPDATE contacts SET name=?2,email=?3,phone=?4,company=?5,notes=?6,favorite=?7,updated=?8,dirty=1 WHERE id=?1",
                params![ct.id, ct.name, ct.email, ct.phone, ct.company, ct.notes, ct.favorite, now()],
            ).map_err(e)?;
            Ok(ct.id)
        } else {
            c.execute(
                "INSERT INTO contacts(name,email,phone,company,notes,favorite,updated,uid,dirty) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1)",
                params![ct.name, ct.email, ct.phone, ct.company, ct.notes, ct.favorite, now(), new_uid()],
            ).map_err(e)?;
            Ok(c.last_insert_rowid())
        }
    }

    /// Synced contacts become tombstones until the server deletion succeeds.
    pub fn delete_contact(&self, id: i64) -> R<()> {
        let c = self.c();
        c.execute("UPDATE contacts SET deleted=1, dirty=1 WHERE id=?1 AND href<>''", [id]).map_err(e)?;
        c.execute("DELETE FROM contacts WHERE id=?1 AND href=''", [id]).map_err(e)?;
        Ok(())
    }

    /// Address autocomplete: saved contacts first, then people seen in mail.
    pub fn suggest(&self, query: &str, limit: usize) -> R<Vec<AddressSuggestion>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(vec![]);
        }
        let like = format!("%{q}%");
        let c = self.c();
        let mut out: Vec<AddressSuggestion> = Vec::new();
        let mut seen = HashSet::new();
        let mut st = c.prepare(
            "SELECT name,email FROM contacts WHERE deleted=0 AND email<>'' AND (name LIKE ?1 OR email LIKE ?1)
             ORDER BY favorite DESC, name COLLATE NOCASE LIMIT 20",
        ).map_err(e)?;
        for r in st.query_map([&like], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(e)?.flatten() {
            if seen.insert(r.1.to_lowercase()) {
                out.push(AddressSuggestion { name: r.0, email: r.1, is_contact: true });
            }
        }
        let mut st = c.prepare(
            "SELECT from_name, from_addr, COUNT(*) n FROM messages WHERE from_addr<>'' AND (from_name LIKE ?1 OR from_addr LIKE ?1)
             GROUP BY lower(from_addr) ORDER BY n DESC LIMIT 20",
        ).map_err(e)?;
        for r in st.query_map([&like], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(e)?.flatten() {
            if seen.insert(r.1.to_lowercase()) {
                out.push(AddressSuggestion { name: r.0, email: r.1, is_contact: false });
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    // ---------- todos ----------
    fn row_to_todo(r: &Row) -> rusqlite::Result<Todo> {
        Ok(Todo {
            id: r.get(0)?, title: r.get(1)?, notes: r.get(2)?, due: r.get(3)?, done: r.get(4)?,
            priority: r.get(5)?, account_id: r.get(6)?, folder: r.get(7)?, uid: r.get(8)?, created: r.get(9)?,
        })
    }

    pub fn todos(&self) -> R<Vec<Todo>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id,title,notes,due,done,priority,account_id,folder,uid,created FROM todos WHERE deleted=0
             ORDER BY done, CASE WHEN due IS NULL THEN 1 ELSE 0 END, due, priority DESC, id DESC",
        ).map_err(e)?;
        let rows = st.query_map([], Self::row_to_todo).map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    pub fn save_todo(&self, t: &Todo) -> R<i64> {
        let c = self.c();
        if t.id > 0 {
            // Changing the due date re-arms the reminder.
            c.execute(
                "UPDATE todos SET title=?2,notes=?3,notified=CASE WHEN due IS ?4 THEN notified ELSE 0 END,due=?4,done=?5,priority=?6,
                 account_id=?7,folder=?8,uid=?9,dirty=1 WHERE id=?1",
                params![t.id, t.title, t.notes, t.due, t.done, t.priority, t.account_id, t.folder, t.uid],
            ).map_err(e)?;
            Ok(t.id)
        } else {
            c.execute(
                "INSERT INTO todos(title,notes,due,done,priority,account_id,folder,uid,created,dirty) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1)",
                params![t.title, t.notes, t.due, t.done, t.priority, t.account_id, t.folder, t.uid, now()],
            ).map_err(e)?;
            Ok(c.last_insert_rowid())
        }
    }

    pub fn delete_todo(&self, id: i64) -> R<()> {
        let c = self.c();
        c.execute("UPDATE todos SET deleted=1, dirty=1 WHERE id=?1 AND href<>''", [id]).map_err(e)?;
        c.execute("DELETE FROM todos WHERE id=?1 AND href=''", [id]).map_err(e)?;
        Ok(())
    }

    // ---------- DAV sync bookkeeping (table = "contacts" | "todos") ----------
    fn tbl(table: &str) -> R<&'static str> {
        match table { "contacts" => Ok("contacts"), "todos" => Ok("todos"), _ => Err("bad table".into()) }
    }

    pub fn sync_rows(&self, table: &str) -> R<Vec<SyncRow>> {
        let t = Self::tbl(table)?;
        // the todos table already has a `uid` column (linked email UID), so DAV UIDs use `uid` only for contacts
        let uid_col = if t == "todos" { "dav_uid" } else { "uid" };
        let c = self.c();
        let sql = format!("SELECT id, {uid_col}, href, etag, raw, dirty, deleted FROM {t}");
        let mut st = c.prepare(&sql).map_err(e)?;
        let rows = st.query_map([], |r| Ok(SyncRow {
            id: r.get(0)?, uid: r.get(1)?, href: r.get(2)?, etag: r.get(3)?, raw: r.get(4)?, dirty: r.get(5)?, deleted: r.get(6)?,
        })).map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    pub fn mark_synced(&self, table: &str, id: i64, uid: &str, href: &str, etag: &str, raw: &str) -> R<()> {
        let t = Self::tbl(table)?;
        let uid_col = if t == "todos" { "dav_uid" } else { "uid" };
        self.c().execute(
            &format!("UPDATE {t} SET {uid_col}=?2, href=?3, etag=?4, raw=?5, dirty=0 WHERE id=?1"),
            params![id, uid, href, etag, raw],
        ).map_err(e)?;
        Ok(())
    }

    pub fn purge(&self, table: &str, id: i64) -> R<()> {
        let t = Self::tbl(table)?;
        self.c().execute(&format!("DELETE FROM {t} WHERE id=?1"), [id]).map_err(e)?;
        Ok(())
    }

    pub fn contact_by_id(&self, id: i64) -> R<Option<Contact>> {
        self.c().query_row(
            "SELECT id,name,email,phone,company,notes,favorite FROM contacts WHERE id=?1", [id],
            |r| Ok(Contact { id: r.get(0)?, name: r.get(1)?, email: r.get(2)?, phone: r.get(3)?, company: r.get(4)?, notes: r.get(5)?, favorite: r.get(6)? }),
        ).optional().map_err(e)
    }

    pub fn todo_by_id(&self, id: i64) -> R<Option<Todo>> {
        self.c().query_row(
            "SELECT id,title,notes,due,done,priority,account_id,folder,uid,created FROM todos WHERE id=?1", [id], Self::row_to_todo,
        ).optional().map_err(e)
    }

    /// Writes a contact that came from the server (not dirty). `id` = existing local row, if any.
    pub fn apply_remote_contact(&self, id: Option<i64>, ct: &Contact, uid: &str, href: &str, etag: &str, raw: &str) -> R<()> {
        let c = self.c();
        match id {
            Some(id) => c.execute(
                "UPDATE contacts SET name=?2,email=?3,phone=?4,company=?5,notes=?6,favorite=?7,uid=?8,href=?9,etag=?10,raw=?11,dirty=0,deleted=0,updated=?12 WHERE id=?1",
                params![id, ct.name, ct.email, ct.phone, ct.company, ct.notes, ct.favorite, uid, href, etag, raw, now()],
            ),
            None => c.execute(
                "INSERT INTO contacts(name,email,phone,company,notes,favorite,uid,href,etag,raw,dirty,deleted,updated) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,0,?11)",
                params![ct.name, ct.email, ct.phone, ct.company, ct.notes, ct.favorite, uid, href, etag, raw, now()],
            ),
        }.map_err(e)?;
        Ok(())
    }

    pub fn apply_remote_todo(&self, id: Option<i64>, t: &Todo, uid: &str, href: &str, etag: &str, raw: &str) -> R<()> {
        let c = self.c();
        match id {
            Some(id) => c.execute(
                "UPDATE todos SET title=?2,notes=?3,notified=CASE WHEN due IS ?4 THEN notified ELSE 0 END,due=?4,done=?5,priority=?6,
                 account_id=COALESCE(?7,account_id),folder=COALESCE(?8,folder),uid=COALESCE(?9,uid),dav_uid=?10,href=?11,etag=?12,raw=?13,dirty=0,deleted=0 WHERE id=?1",
                params![id, t.title, t.notes, t.due, t.done, t.priority, t.account_id, t.folder, t.uid, uid, href, etag, raw],
            ),
            None => c.execute(
                "INSERT INTO todos(title,notes,due,done,priority,account_id,folder,uid,created,dav_uid,href,etag,raw,dirty,deleted,notified)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,0,0,?14)",
                params![t.title, t.notes, t.due, t.done, t.priority, t.account_id, t.folder, t.uid, if t.created > 0 { t.created } else { now() },
                        uid, href, etag, raw, t.due.map(|d| d <= now()).unwrap_or(false)],
            ),
        }.map_err(e)?;
        Ok(())
    }

    /// Ensures every row has a DAV UID (rows created before sync was enabled).
    pub fn ensure_uids(&self) -> R<()> {
        let c = self.c();
        for (t, col) in [("contacts", "uid"), ("todos", "dav_uid")] {
            let ids: Vec<i64> = {
                let mut st = c.prepare(&format!("SELECT id FROM {t} WHERE {col}=''")).map_err(e)?;
                let it = st.query_map([], |r| r.get(0)).map_err(e)?;
                it.flatten().collect()
            };
            for id in ids {
                c.execute(&format!("UPDATE {t} SET {col}=?2 WHERE id=?1"), params![id, new_uid()]).map_err(e)?;
            }
        }
        Ok(())
    }

    /// Due, unfinished todos that have not been announced yet; marks them announced.
    pub fn take_due_todos(&self) -> R<Vec<Todo>> {
        let mut c = self.c();
        let tx = c.transaction().map_err(e)?;
        let list = {
            let mut st = tx.prepare(
                "SELECT id,title,notes,due,done,priority,account_id,folder,uid,created FROM todos
                 WHERE deleted=0 AND done=0 AND notified=0 AND due IS NOT NULL AND due<=?1",
            ).map_err(e)?;
            let rows = st.query_map([now()], Self::row_to_todo).map_err(e)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)?
        };
        for t in &list {
            tx.execute("UPDATE todos SET notified=1 WHERE id=?1", [t.id]).map_err(e)?;
        }
        tx.commit().map_err(e)?;
        Ok(list)
    }

    // ---------- local draft copies ----------
    pub fn save_local_draft(&self, id: Option<i64>, json: &str, pending: bool) -> R<i64> {
        let c = self.c();
        match id {
            Some(id) if id > 0 => {
                let n = c.execute("UPDATE drafts SET json=?2, pending=?3, updated=?4 WHERE id=?1", params![id, json, pending, now()]).map_err(e)?;
                if n > 0 {
                    return Ok(id);
                }
                c.execute("INSERT INTO drafts(id,json,pending,updated) VALUES(?1,?2,?3,?4)", params![id, json, pending, now()]).map_err(e)?;
                Ok(id)
            }
            _ => {
                c.execute("INSERT INTO drafts(json,pending,updated) VALUES(?1,?2,?3)", params![json, pending, now()]).map_err(e)?;
                Ok(c.last_insert_rowid())
            }
        }
    }
    pub fn delete_local_draft(&self, id: i64) {
        let _ = self.c().execute("DELETE FROM drafts WHERE id=?1", [id]);
    }
    pub fn pending_drafts(&self) -> R<Vec<(i64, String)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id,json FROM drafts WHERE pending=1 ORDER BY updated DESC").map_err(e)?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    pub fn summaries_for(&self, account_id: i64, folder: &str, uids: &[u32]) -> R<Vec<MsgSummary>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT {SUMMARY_COLS} FROM messages WHERE account_id=?1 AND folder=?2 AND uid=?3")).map_err(e)?;
        let mut out = Vec::with_capacity(uids.len());
        for uid in uids {
            if let Ok(m) = st.query_row(params![account_id, folder, uid], row_to_summary) {
                out.push(m);
            }
        }
        Ok(out)
    }

    pub fn search(&self, account_id: Option<i64>, query: &str, limit: u32) -> R<Vec<MsgSummary>> {
        let c = self.c();
        let like = format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let mut st = c.prepare(&format!(
            "SELECT {SUMMARY_COLS} FROM messages
             WHERE (?1 IS NULL OR account_id=?1)
               AND (subject LIKE ?2 ESCAPE '\\' OR from_name LIKE ?2 ESCAPE '\\' OR from_addr LIKE ?2 ESCAPE '\\'
                    OR to_addrs LIKE ?2 ESCAPE '\\' OR body_text LIKE ?2 ESCAPE '\\')
             ORDER BY date DESC LIMIT ?3"
        )).map_err(e)?;
        let rows = st.query_map(params![account_id, like, limit], row_to_summary).map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    // ---------- fallback secret store ----------
    pub fn set_secret(&self, key: &str, value: &str) -> R<()> {
        self.c().execute(
            "INSERT INTO secrets(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2",
            params![key, value],
        ).map_err(e)?;
        Ok(())
    }
    pub fn get_secret(&self, key: &str) -> Option<String> {
        self.c().query_row("SELECT value FROM secrets WHERE key=?1", [key], |r| r.get(0)).ok()
    }
    pub fn del_secret(&self, key: &str) {
        let _ = self.c().execute("DELETE FROM secrets WHERE key=?1", [key]);
    }
}

impl Db {
    /// Per-account options saved by the UI (app settings → account_prefs.<id>).
    pub fn account_prefs(&self, account_id: i64) -> crate::models::AccountPrefs {
        self.get_setting("app")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("account_prefs").and_then(|p| p.get(account_id.to_string())).cloned())
            .and_then(|p| serde_json::from_value(p).ok())
            .unwrap_or_default()
    }
}
