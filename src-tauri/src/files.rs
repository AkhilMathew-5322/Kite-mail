//! The "Files" folder: a real IMAP folder next to Inbox and Sent that collects documents and
//! pictures. Each file is stored as one small message with the file attached, so it is visible
//! (and syncs) in every mail app – Outlook, Thunderbird, phones – not only in Kite Mail.
//! Kite Mail shows the folder as a gallery and can file attachments there automatically.
//! Duplicates are detected on the server with an X-Kite-File-Hash header.

use crate::archive::{build_eml, OutAtt, OutMsg};
use crate::mail::AppState;
use crate::models::Account;
use mail_parser::{MessageParser, MimeHeaders};
use sha2::{Digest, Sha256};

pub const FOLDER: &str = "Files";
const HASH_HEADER: &str = "X-Kite-File-Hash";

/// Kinds of files that can be collected automatically.
pub fn category(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "bmp" | "tif" | "tiff" => "images",
        "pdf" => "pdf",
        "doc" | "docx" | "odt" | "rtf" | "pages" => "word",
        "xls" | "xlsx" | "ods" | "csv" | "numbers" => "excel",
        "ppt" | "pptx" | "odp" | "key" => "powerpoint",
        _ => "other",
    }
}

/// Where a file came from (shown in the gallery and in other mail apps).
#[derive(Default, Clone)]
pub struct Source {
    pub subject: String,
    pub from: String,
    pub date: i64,
}

pub struct FileItem {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
    pub source: Option<Source>,
}

fn hash(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// The message that carries one file.
pub fn file_message(acct: &Account, item: &FileItem) -> Vec<u8> {
    let me = if acct.name.is_empty() { acct.email.clone() } else { format!("\"{}\" <{}>", acct.name.replace('"', "'"), acct.email) };
    let mut text = format!("📎 {}  ({})\r\n\r\nSaved to the Files folder by Kite Mail.", item.name, human_size(item.data.len()));
    if let Some(s) = &item.source {
        text.push_str(&format!("\r\n\r\nFrom the email: {}\r\nSent by: {}", s.subject, s.from));
        if s.date > 0 {
            if let Some(d) = chrono::DateTime::from_timestamp(s.date, 0) {
                text.push_str(&format!("\r\nDate: {}", d.format("%d %b %Y %H:%M UTC")));
            }
        }
    }
    let m = OutMsg {
        from: me.clone(),
        to: me,
        subject: item.name.clone(),
        date: item.source.as_ref().map(|s| s.date).filter(|d| *d > 0).unwrap_or_else(|| chrono::Utc::now().timestamp()),
        message_id: format!("<file-{}@kitemail>", &hash(&item.data)[..24]),
        text,
        attachments: vec![OutAtt { name: item.name.clone(), mime: item.mime.clone(), data: item.data.clone(), content_id: String::new() }],
        ..Default::default()
    };
    let raw = build_eml(&m);
    // our private header goes first so other apps ignore it
    let mut out = format!("{HASH_HEADER}: {}\r\nX-Kite-File: 1\r\n", hash(&item.data)).into_bytes();
    out.extend_from_slice(&raw);
    out
}

fn human_size(n: usize) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / 1_048_576.0),
        n if n >= 1 << 10 => format!("{} KB", n / 1024),
        n => format!("{n} bytes"),
    }
}

/// Attachments worth filing: real attachments (not logos embedded in the text) of the wanted kinds.
pub fn collectable(raw: &[u8], kinds: &[String]) -> Vec<FileItem> {
    let Some(m) = MessageParser::default().parse(raw) else { return vec![] };
    let src = Source {
        subject: m.subject().unwrap_or("").to_string(),
        from: m.from().and_then(|a| a.first()).map(|a| a.address().unwrap_or("").to_string()).unwrap_or_default(),
        date: m.date().map(|d| d.to_timestamp()).unwrap_or(0),
    };
    let mut out = Vec::new();
    for part in m.attachments() {
        let Some(name) = part.attachment_name().map(String::from) else { continue };
        let kind = category(&name);
        if !kinds.iter().any(|k| k == kind) {
            continue;
        }
        let data = part.contents();
        let inline_logo = part.content_id().is_some() && part.content_disposition().map(|d| !d.is_attachment()).unwrap_or(true);
        if inline_logo || (kind == "images" && data.len() < 15 * 1024) || data.len() > 25 * 1024 * 1024 {
            continue;
        }
        let mime = part.content_type().map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream"))).unwrap_or_default();
        out.push(FileItem { name, mime, data: data.to_vec(), source: Some(src.clone()) });
    }
    out
}

impl AppState {
    /// The account's Files folder, created on the server the first time.
    pub fn files_folder(&self, acct: &Account) -> Result<String, String> {
        if let Some(f) = self.db.folder_by_special(acct.id, "files")? {
            return Ok(f);
        }
        // Servers like cPanel/Courier keep every folder under "INBOX." and refuse top-level
        // names, so fall back to INBOX<delim>Files when the plain name is rejected.
        let delim = self
            .with_imap(acct, |c| Ok(c.sess.list(Some(""), Some("INBOX"))?.iter().find_map(|n| n.delimiter().map(str::to_string))))?
            .unwrap_or_else(|| "/".into());
        let mut last_err = String::new();
        for name in [FOLDER.to_string(), format!("INBOX{delim}{FOLDER}")] {
            match self.with_imap(acct, |c| c.sess.create(&name)) {
                Ok(()) => {}
                Err(e) if already_exists(&e) => {}
                Err(e) => {
                    last_err = e;
                    continue;
                }
            }
            let _ = self.with_imap(acct, |c| c.sess.subscribe(&name));
            self.refresh_folders(acct)?;
            if let Some(f) = self.db.folder_by_special(acct.id, "files")? {
                return Ok(f);
            }
        }
        self.refresh_folders(acct)?;
        self.db.folder_by_special(acct.id, "files")?.ok_or_else(|| {
            if last_err.is_empty() {
                "Could not create the Files folder on the mail server".to_string()
            } else {
                format!("Could not create the Files folder on the mail server: {last_err}")
            }
        })
    }

    /// Uploads files to the Files folder, skipping ones already there. Returns (added, already there).
    pub fn save_files(&self, acct: &Account, items: &[FileItem]) -> Result<(u32, u32), String> {
        if items.is_empty() {
            return Ok((0, 0));
        }
        let folder = self.files_folder(acct)?;
        let (mut added, mut skipped) = (0, 0);
        self.with_imap_once(acct, |c| {
            c.sess.select(&folder)?;
            c.selected = Some(folder.clone());
            for it in items {
                let h = hash(&it.data);
                if !c.sess.uid_search(format!("HEADER {HASH_HEADER} {h}"))?.is_empty() {
                    skipped += 1;
                    continue;
                }
                c.sess.append(&folder, &file_message(acct, it)).flag(imap::types::Flag::Seen).finish()?;
                added += 1;
            }
            Ok(())
        })?;
        let _ = self.sync_folder(acct, &folder, 500);
        Ok((added, skipped))
    }

    /// Files attachments from Inbox mail of the last `days` days (one-off catch-up).
    pub fn collect_recent(&self, acct: &Account, days: u32) -> Result<u32, String> {
        let inbox = self.db.folder_by_special(acct.id, "inbox")?.unwrap_or_else(|| "INBOX".into());
        let since = chrono::Utc::now().timestamp() - days.clamp(1, 365) as i64 * 86_400;
        let uids: Vec<u32> = self.db.list_messages(acct.id, &inbox, 5000)?.into_iter().filter(|m| m.has_attach && m.date >= since).map(|m| m.uid).collect();
        let kinds = self.db.account_prefs(acct.id).collect_kinds;
        let mut items = Vec::new();
        for uid in uids {
            if let Ok(raw) = self.raw_message(acct, &inbox, uid) {
                items.extend(collectable(&raw, &kinds));
            }
        }
        if items.is_empty() {
            self.files_folder(acct)?; // still make the folder visible
            return Ok(0);
        }
        self.save_files(acct, &items).map(|(a, _)| a)
    }

    /// Run once per account when it connects: makes sure the Files folder exists (so it shows
    /// next to Inbox and Sent right away) and, the first time ever, fills it from the last
    /// 30 days of mail. Returns how many files were added.
    pub fn prepare_files(&self, acct: &Account) -> Result<u32, String> {
        if !self.db.account_prefs(acct.id).collect_files {
            return Ok(0);
        }
        self.files_folder(acct)?;
        let key = format!("files-initial:{}", acct.email.to_lowercase());
        if self.db.get_setting(&key).is_some() {
            return Ok(0);
        }
        let n = self.collect_recent(acct, 30)?;
        self.db.set_setting(&key, "1")?;
        Ok(n)
    }

    /// Files new mail's attachments (account setting "collect_files").
    pub fn collect_from(&self, acct: &Account, folder: &str, uids: &[u32]) -> Result<u32, String> {
        let prefs = self.db.account_prefs(acct.id);
        if !prefs.collect_files || prefs.collect_kinds.is_empty() {
            return Ok(0);
        }
        let mut items = Vec::new();
        for &uid in uids {
            if let Ok(raw) = self.raw_message(acct, folder, uid) {
                items.extend(collectable(&raw, &prefs.collect_kinds));
            }
        }
        self.save_files(acct, &items).map(|(a, _)| a)
    }
}

/// True for a CREATE refusal that only means the folder is already there.
fn already_exists(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("alreadyexists") || e.contains("already exist") || e.contains("duplicate")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct() -> Account {
        Account { id: 1, name: "Akhil".into(), email: "akhil@example.ae".into(), kind: "imap".into(), imap_host: String::new(), imap_port: 993,
            smtp_host: String::new(), smtp_port: 465, smtp_security: "ssl".into(), username: String::new(), tenant: String::new(),
            client_id: String::new(), client_secret: String::new(), save_sent: true }
    }

    #[test]
    fn picks_real_attachments_of_wanted_kinds() {
        use base64::Engine;
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let big = vec![7u8; 40_000];
        let raw = format!(
            "From: Sara <sara@x.ae>\r\nSubject: Contract\r\nDate: Mon, 3 Nov 2025 10:00:00 +0400\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
             --b\r\nContent-Type: text/html\r\n\r\n<img src=cid:logo>\r\n\
             --b\r\nContent-Type: image/png; name=logo.png\r\nContent-ID: <logo>\r\nContent-Disposition: inline; filename=logo.png\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n\
             --b\r\nContent-Type: application/pdf; name=contract.pdf\r\nContent-Disposition: attachment; filename=contract.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n\
             --b\r\nContent-Type: image/jpeg; name=tiny.jpg\r\nContent-Disposition: attachment; filename=tiny.jpg\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n\
             --b\r\nContent-Type: image/jpeg; name=site-photo.jpg\r\nContent-Disposition: attachment; filename=site-photo.jpg\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n\
             --b\r\nContent-Type: application/vnd.ms-excel; name=costs.xlsx\r\nContent-Disposition: attachment; filename=costs.xlsx\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n--b--\r\n",
            b64(&big), b64(b"%PDF-1.4 contract"), b64(&[1u8; 900]), b64(&big), b64(b"xlsx")
        );
        let kinds = vec!["images".to_string(), "pdf".to_string(), "word".to_string()];
        let got: Vec<String> = collectable(raw.as_bytes(), &kinds).into_iter().map(|f| f.name).collect();
        assert_eq!(got, vec!["contract.pdf", "site-photo.jpg"]); // no inline logo, no tiny image, no excel
        let item = collectable(raw.as_bytes(), &kinds).remove(0);
        let msg = file_message(&acct(), &item);
        let m = MessageParser::default().parse(&msg).unwrap();
        assert_eq!(m.subject(), Some("contract.pdf"));
        assert!(m.header_raw(HASH_HEADER).is_some());
        assert_eq!(m.attachments().next().unwrap().contents(), b"%PDF-1.4 contract");
        assert!(m.body_text(0).unwrap().contains("From the email: Contract"));
        assert_eq!(category("Scan.JPEG"), "images");
        assert_eq!(category("brief.docx"), "word");
    }
}
