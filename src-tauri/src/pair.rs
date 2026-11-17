//! Phone pairing: the desktop packs accounts (with their passwords / sign-in tokens), contacts,
//! tasks, settings and the Nextcloud connection into one AES-256-GCM encrypted blob, serves it
//! over the local network for 10 minutes (any number of phones, until stopped), and shows a QR
//! code containing the address and the key. Only devices that can see the QR code can decrypt. The phone scans, downloads, decrypts and imports. Mail then syncs from the
//! mail servers – it is never copied.

use crate::db::Db;
use crate::models::{Contact, Todo};
use crate::{nextcloud, secrets};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const PAIR_SECONDS: u64 = 600;

#[derive(Serialize, Deserialize, Debug)]
pub struct PairAccount {
    pub name: String,
    pub email: String,
    pub kind: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: String,
    pub username: String,
    pub tenant: String,
    pub client_id: String,
    pub client_secret: String,
    pub save_sent: bool,
    /// Password (IMAP) or OAuth refresh token (Microsoft 365 / Gmail).
    pub secret: String,
    pub signature: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Payload {
    pub v: u32,
    pub created: i64,
    pub accounts: Vec<PairAccount>,
    pub contacts: Vec<Contact>,
    pub todos: Vec<Todo>,
    pub settings: serde_json::Value,
    pub nextcloud: Option<(nextcloud::NcConfig, String)>,
}

#[derive(Serialize, Debug, Default)]
pub struct ImportReport {
    pub accounts: u32,
    pub skipped_accounts: u32,
    pub contacts: u32,
    pub todos: u32,
    pub nextcloud: bool,
}

/// Progress of an active pairing code, sent to the UI.
#[derive(Serialize, Debug, Clone)]
pub struct PairEvent {
    /// "paired" (a device downloaded the setup) | "ended" (expired or stopped)
    pub kind: String,
    pub count: u32,
}

#[derive(Serialize, Debug)]
pub struct PairInfo {
    pub code: String,
    pub qr_svg: String,
    pub expires_in: u64,
    pub accounts: usize,
}

// ------------------------------------------------------------------ build / import

pub fn build_payload(db: &Db) -> Result<Payload, String> {
    let settings: serde_json::Value = db.get_setting("app").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| serde_json::json!({}));
    let sigs = settings.get("signatures").cloned().unwrap_or_default();
    let mut accounts = Vec::new();
    for a in db.accounts()? {
        let key = if a.is_oauth() { secrets::refresh_key(a.id) } else { secrets::password_key(a.id) };
        let secret = secrets::get(db, &key).unwrap_or_default();
        accounts.push(PairAccount {
            signature: sigs.get(a.id.to_string()).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            name: a.name, email: a.email, kind: a.kind, imap_host: a.imap_host, imap_port: a.imap_port,
            smtp_host: a.smtp_host, smtp_port: a.smtp_port, smtp_security: a.smtp_security, username: a.username,
            tenant: a.tenant, client_id: a.client_id, client_secret: a.client_secret, save_sent: a.save_sent, secret,
        });
    }
    let nc = nextcloud::load(db);
    let nextcloud = if nc.enabled {
        secrets::get(db, nextcloud::PASSWORD_KEY).map(|pw| (nextcloud::NcConfig { last_backup: Default::default(), ..nc }, pw))
    } else {
        None
    };
    let mut settings = settings;
    if let Some(o) = settings.as_object_mut() {
        o.remove("signatures"); // re-keyed per account on import
    }
    Ok(Payload {
        v: 1,
        created: chrono::Utc::now().timestamp(),
        accounts,
        contacts: db.contacts("")?,
        todos: db.todos()?.into_iter().filter(|t| !t.done).collect(),
        settings,
        nextcloud,
    })
}

pub fn import_payload(db: &Db, p: Payload) -> Result<ImportReport, String> {
    let mut rep = ImportReport::default();
    let existing: Vec<String> = db.accounts()?.into_iter().map(|a| a.email.to_lowercase()).collect();
    let mut app: serde_json::Value = db.get_setting("app").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| serde_json::json!({}));

    // settings: take the desktop's look & preferences, keep anything device-specific already set here
    if let (Some(dst), Some(src)) = (app.as_object_mut(), p.settings.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    let mut sigs = app.get("signatures").cloned().unwrap_or_else(|| serde_json::json!({}));

    for a in p.accounts {
        if existing.contains(&a.email.to_lowercase()) {
            rep.skipped_accounts += 1;
            continue;
        }
        let acct = crate::models::Account {
            id: 0, name: a.name, email: a.email, kind: a.kind, imap_host: a.imap_host, imap_port: a.imap_port,
            smtp_host: a.smtp_host, smtp_port: a.smtp_port, smtp_security: a.smtp_security, username: a.username,
            tenant: a.tenant, client_id: a.client_id, client_secret: a.client_secret, save_sent: a.save_sent,
        };
        let id = db.insert_account(&acct)?;
        let key = if acct.is_oauth() { secrets::refresh_key(id) } else { secrets::password_key(id) };
        if !a.secret.is_empty() {
            secrets::set(db, &key, &a.secret)?;
        }
        if !a.signature.is_empty() {
            sigs[id.to_string()] = serde_json::Value::String(a.signature);
        }
        rep.accounts += 1;
    }
    app["signatures"] = sigs;
    db.set_setting("app", &app.to_string())?;

    // With Nextcloud sync on, contacts/tasks arrive from the server – importing them too would duplicate.
    let nc_handles = |contacts: bool| p.nextcloud.as_ref().map(|(c, _)| c.enabled && if contacts { c.sync_contacts } else { c.sync_tasks }).unwrap_or(false);
    let local_nc = nextcloud::load(db);
    if let Some((cfg, pw)) = &p.nextcloud {
        if !local_nc.enabled {
            nextcloud::save(db, cfg)?;
            secrets::set(db, nextcloud::PASSWORD_KEY, pw)?;
            rep.nextcloud = true;
        }
    }
    if !nc_handles(true) {
        let mine = db.contacts("")?;
        for c in p.contacts {
            let dup = mine.iter().any(|m| (!c.email.is_empty() && m.email.eq_ignore_ascii_case(&c.email)) || (c.email.is_empty() && m.name == c.name && m.phone == c.phone));
            if !dup {
                db.save_contact(&Contact { id: 0, ..c })?;
                rep.contacts += 1;
            }
        }
    }
    if !nc_handles(false) {
        let mine = db.todos()?;
        for t in p.todos {
            if !mine.iter().any(|m| m.title == t.title && m.due == t.due) {
                // email links refer to the desktop's account ids – drop them
                db.save_todo(&Todo { id: 0, account_id: None, folder: None, uid: None, ..t })?;
                rep.todos += 1;
            }
        }
    }
    Ok(rep)
}

// ------------------------------------------------------------------ crypto

pub fn encrypt(key: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(Nonce::from_slice(&nonce), plain).expect("encrypt"));
    out
}

pub fn decrypt(key: &[u8], blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < 28 || key.len() != 32 {
        return Err("Pairing data is incomplete".into());
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    cipher.decrypt(Nonce::from_slice(&blob[..12]), &blob[12..]).map_err(|_| "Pairing code doesn't match – scan the QR code again".to_string())
}

// ------------------------------------------------------------------ transport

/// The address other devices on this network can reach us at.
/// Candidate addresses the phone may reach this computer on (Wi-Fi, Ethernet, VPN…).
/// Asks the OS which local address it would use to reach each private range and the
/// internet; no packets are sent. At most 4, most likely first.
pub fn lan_ips() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for target in ["10.255.255.255:9", "192.168.255.255:9", "172.31.255.255:9", "1.1.1.1:9"] {
        let ip = UdpSocket::bind("0.0.0.0:0").and_then(|s| { s.connect(target)?; s.local_addr() }).map(|a| a.ip());
        if let Ok(std::net::IpAddr::V4(v4)) = ip {
            let txt = v4.to_string();
            if !v4.is_unspecified() && !v4.is_loopback() && !out.contains(&txt) {
                out.push(txt);
            }
        }
    }
    // Private (home/office LAN) addresses before anything else.
    out.sort_by_key(|ip| !ip.parse::<std::net::Ipv4Addr>().map(|a| a.is_private()).unwrap_or(false));
    out.truncate(4);
    out
}

pub fn qr_svg(text: &str) -> String {
    use qrcode::render::svg;
    match qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::M) {
        Ok(code) => code.render::<svg::Color>().min_dimensions(260, 260).quiet_zone(true)
            .dark_color(svg::Color("#111111")).light_color(svg::Color("#ffffff")).build(),
        Err(e) => format!("<p>QR error: {e}</p>"),
    }
}

/// Starts the pairing server. The code stays valid for PAIR_SECONDS and can be used by several
/// phones (or to re-pair the same phone) until it expires or `stop` is set.
pub fn start(db: &Db, host_override: Option<String>, stop: Arc<AtomicBool>, on_event: impl Fn(PairEvent) + Send + 'static) -> Result<PairInfo, String> {
    let payload = build_payload(db)?;
    let accounts = payload.accounts.len();
    let json = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let mut tok = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut tok);
    let token = URL_SAFE_NO_PAD.encode(tok);
    let blob = encrypt(&key, &json);

    let listener = TcpListener::bind("0.0.0.0:0").map_err(|e| format!("Cannot open a network port: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let hosts = match host_override {
        Some(h) => vec![h],
        None => {
            let ips = lan_ips();
            if ips.is_empty() { vec!["127.0.0.1".to_string()] } else { ips }
        }
    };
    let h = hosts.iter().map(|ip| format!("{ip}:{port}")).collect::<Vec<_>>().join(",");
    let code = format!("kitemail://pair?h={h}&t={token}&k={}", URL_SAFE_NO_PAD.encode(key));

    std::thread::Builder::new().name("pairing".into()).spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(PAIR_SECONDS);
        let path = format!("/pair/{token}");
        let mut count = 0u32;
        while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
            let Ok((mut s, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            };
            let _ = s.set_nonblocking(false);
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            if req.split_whitespace().nth(1) == Some(path.as_str()) {
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", blob.len());
                let _ = s.write_all(&blob);
                let _ = s.flush();
                count += 1;
                on_event(PairEvent { kind: "paired".into(), count });
            } else {
                let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        }
        on_event(PairEvent { kind: "ended".into(), count });
    }).map_err(|e| e.to_string())?;

    Ok(PairInfo { qr_svg: qr_svg(&code), code, expires_in: PAIR_SECONDS, accounts })
}

/// Phone side: download, decrypt and import from a scanned/pasted pairing code.
pub fn import_from_code(db: &Db, code: &str) -> Result<ImportReport, String> {
    let u = url::Url::parse(code.trim()).map_err(|_| "That isn't a Kite Mail pairing code".to_string())?;
    if u.scheme() != "kitemail" {
        return Err("That isn't a Kite Mail pairing code".into());
    }
    let q = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
    let (host, token, key) = match (q("h"), q("t"), q("k")) {
        (Some(h), Some(t), Some(k)) => (h, t, k),
        _ => return Err("The pairing code is incomplete".into()),
    };
    let key = URL_SAFE_NO_PAD.decode(key).map_err(|_| "The pairing code is damaged".to_string())?;
    // The computer lists every address it might be reachable on; use the first that answers.
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(4)).timeout(Duration::from_secs(30)).build();
    let mut last_err = String::new();
    let mut resp = None;
    for h in host.split(',').map(str::trim).filter(|h| !h.is_empty()).take(6) {
        match agent.get(&format!("http://{h}/pair/{token}")).call() {
            Ok(r) => { resp = Some(r); break; }
            Err(ureq::Error::Status(404, _)) => return Err("This pairing code is not valid any more – show a new QR code on the computer".into()),
            Err(other) => last_err = other.to_string(),
        }
    }
    let resp = resp.ok_or_else(|| format!("Can't reach the computer ({last_err}). Make sure both devices are on the same Wi-Fi and the QR code is still showing (codes expire after 10 minutes)."))?;
    let mut blob = Vec::new();
    resp.into_reader().take(50 << 20).read_to_end(&mut blob).map_err(|e| e.to_string())?;
    let plain = decrypt(&key, &blob)?;
    let payload: Payload = serde_json::from_slice(&plain).map_err(|e| format!("Pairing data not understood: {e}"))?;
    import_payload(db, payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn crypto_roundtrip_and_tamper() {
        let key = [7u8; 32];
        let blob = encrypt(&key, b"secret stuff");
        assert_eq!(decrypt(&key, &blob).unwrap(), b"secret stuff");
        assert!(decrypt(&[8u8; 32], &blob).is_err(), "wrong key rejected");
        let mut bad = blob.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(decrypt(&key, &bad).is_err(), "tampering detected");
    }

    #[test]
    fn pair_over_network_end_to_end() {
        // ---- desktop
        let desk = Db::memory();
        let acct = crate::models::Account {
            id: 0, name: "Akhil".into(), email: "akhil@kb.com".into(), kind: "m365".into(), imap_host: "outlook.office365.com".into(),
            imap_port: 993, smtp_host: "smtp.office365.com".into(), smtp_port: 587, smtp_security: "starttls".into(),
            username: "akhil@kb.com".into(), tenant: "kb.com".into(), client_id: "cid".into(), client_secret: String::new(), save_sent: false,
        };
        let id = desk.insert_account(&acct).unwrap();
        secrets::set(&desk, &secrets::refresh_key(id), "REFRESH-TOKEN-XYZ").unwrap();
        desk.set_setting("app", &serde_json::json!({"light_theme": "ocean", "signatures": {id.to_string(): "Akhil · TLR"}}).to_string()).unwrap();
        desk.save_contact(&Contact { name: "Sara Khan".into(), email: "sara@kb.com".into(), phone: "+971".into(), ..Default::default() }).unwrap();
        desk.save_todo(&Todo { title: "Call Ahmed".into(), due: Some(2_000_000_000), ..Default::default() }).unwrap();
        desk.save_todo(&Todo { title: "Old done".into(), done: true, ..Default::default() }).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let info = start(&desk, Some("127.0.0.1".into()), stop.clone(), move |e| { tx.send(e).unwrap(); }).unwrap();
        assert!(info.qr_svg.starts_with("<?xml") || info.qr_svg.contains("<svg"));
        assert!(info.code.starts_with("kitemail://pair?h=127.0.0.1:"));

        // ---- phone (already has its own contact Sara -> no duplicate)
        let phone = Db::memory();
        phone.save_contact(&Contact { name: "Sara".into(), email: "SARA@kb.com".into(), ..Default::default() }).unwrap();
        let rep = import_from_code(&phone, &info.code).unwrap();
        let ev = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((ev.kind.as_str(), ev.count), ("paired", 1), "desktop notified");
        assert_eq!((rep.accounts, rep.contacts, rep.todos), (1, 0, 1));
        let a = &phone.accounts().unwrap()[0];
        assert_eq!((a.email.as_str(), a.tenant.as_str()), ("akhil@kb.com", "kb.com"));
        assert_eq!(secrets::get(&phone, &secrets::refresh_key(a.id)).as_deref(), Some("REFRESH-TOKEN-XYZ"));
        let app: serde_json::Value = serde_json::from_str(&phone.get_setting("app").unwrap()).unwrap();
        assert_eq!(app["light_theme"], "ocean");
        assert_eq!(app["signatures"][a.id.to_string()], "Akhil · TLR");
        assert_eq!(phone.todos().unwrap().len(), 1, "completed tasks are not copied");

        // ---- the same code works again (re-pair / second phone) and never duplicates
        let again = import_from_code(&phone, &info.code).unwrap();
        assert_eq!((again.accounts, again.skipped_accounts, again.todos), (0, 1, 0));
        let second_phone = Db::memory();
        // QR lists several addresses: an unreachable one first must not stop the import
        let multi = info.code.replacen("h=", "h=127.0.0.1:9,", 1);
        assert_eq!(import_from_code(&second_phone, &multi).unwrap().accounts, 1);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().count, 2);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().count, 3);
        // ---- stop: the code no longer works
        stop.store(true, Ordering::Relaxed);
        let ended = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((ended.kind.as_str(), ended.count), ("ended", 3));
        assert!(import_from_code(&Db::memory(), &info.code).unwrap_err().contains("Can't reach"));
        // a wrong token is refused even while the code is active
        let stop2 = Arc::new(AtomicBool::new(false));
        let info2 = start(&desk, Some("127.0.0.1".into()), stop2.clone(), |_| {}).unwrap();
        let forged = info2.code.replace("&t=", "&t=x");
        assert!(import_from_code(&Db::memory(), &forged).unwrap_err().contains("not valid"));
        stop2.store(true, Ordering::Relaxed);
    }
}
