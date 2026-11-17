//! Builds and sends outgoing mail with lettre.

use crate::mail::Auth;
use crate::models::{Account, Draft};
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{Message, SmtpTransport, Transport};
use std::time::Duration;

fn parse_list(s: &str) -> Result<Vec<Mailbox>, String> {
    split_addresses(s)
        .into_iter()
        .map(|a| a.parse::<Mailbox>().map_err(|_| format!("\"{a}\" is not a valid email address – check the To, Cc and Bcc fields")))
        .collect()
}

/// Splits "A <a@x>, b@y; \"Last, First\" <c@z>" on commas/semicolons outside quotes and angle brackets.
pub fn split_addresses(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut in_q, mut in_a) = (false, false);
    for ch in s.chars() {
        match ch {
            '"' => { in_q = !in_q; cur.push(ch) }
            '<' if !in_q => { in_a = true; cur.push(ch) }
            '>' if !in_q => { in_a = false; cur.push(ch) }
            ',' | ';' if !in_q && !in_a => {
                if !cur.trim().is_empty() { out.push(cur.trim().to_string()) }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Plain text -> simple HTML (quoted reply lines rendered as a blockquote style).
pub fn text_to_html(body: &str) -> String {
    let mut html = String::from("<div style=\"font-family:Segoe UI,Arial,sans-serif;font-size:14px\">");
    for line in body.lines() {
        if line.starts_with('>') {
            html.push_str(&format!("<span style=\"color:#555;border-left:3px solid #ccc;padding-left:6px\">{}</span><br>", html_escape(line.trim_start_matches(['>', ' ']))));
        } else {
            html.push_str(&html_escape(line));
            html.push_str("<br>");
        }
    }
    html.push_str("</div>");
    html
}

fn guess_mime(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "eml" => "message/rfc822",
        _ => "application/octet-stream",
    }
}

pub const DRAFT_HEADER: &str = "X-KiteMail-Draft";

pub fn build(acct: &Account, d: &Draft, forwarded: Vec<(String, String, Vec<u8>)>) -> Result<Message, String> {
    build_inner(acct, d, forwarded, false)
}

/// Builds a draft for the Drafts folder: recipients may be empty or incomplete; the exact
/// recipient text is kept in an X-KiteMail-Draft header so it can be restored when re-opened.
pub fn build_draft(acct: &Account, d: &Draft, forwarded: Vec<(String, String, Vec<u8>)>) -> Result<Message, String> {
    build_inner(acct, d, forwarded, true)
}

fn build_inner(acct: &Account, d: &Draft, forwarded: Vec<(String, String, Vec<u8>)>, draft: bool) -> Result<Message, String> {
    use base64::Engine;
    let from_addr: lettre::Address = acct.email.parse().map_err(|e| format!("Bad sender address: {e}"))?;
    let from = Mailbox::new(Some(acct.name.clone()).filter(|n| !n.is_empty()), from_addr.clone());
    let lenient = |s: &str| -> Vec<Mailbox> { split_addresses(s).iter().filter_map(|a| a.parse().ok()).collect() };
    let (to, cc, bcc) = if draft {
        (lenient(&d.to), lenient(&d.cc), lenient(&d.bcc))
    } else {
        (parse_list(&d.to)?, parse_list(&d.cc)?, parse_list(&d.bcc)?)
    };
    if !draft && to.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err("Add at least one recipient".into());
    }

    let mut b = Message::builder().from(from).subject(d.subject.clone()).user_agent("Kite Mail".into());
    b = match d.message_id.as_ref().filter(|m| !m.is_empty()) {
        Some(m) => b.message_id(Some(m.clone())),
        None => b.message_id(None),
    };
    if draft {
        let raw = serde_json::json!({ "to": d.to, "cc": d.cc, "bcc": d.bcc }).to_string();
        let enc = base64::engine::general_purpose::STANDARD.encode(raw);
        b = b
            .keep_bcc()
            .envelope(lettre::address::Envelope::new(Some(from_addr.clone()), vec![from_addr]).map_err(|e| e.to_string())?)
            .raw_header(lettre::message::header::HeaderValue::new(
                lettre::message::header::HeaderName::new_from_ascii_str(DRAFT_HEADER),
                enc,
            ));
    }
    for m in to { b = b.to(m); }
    for m in cc { b = b.cc(m); }
    for m in bcc { b = b.bcc(m); }
    if let Some(irt) = d.in_reply_to.as_ref().filter(|s| !s.is_empty()) {
        b = b.in_reply_to(irt.clone());
        let refs = match d.references.as_ref().filter(|s| !s.is_empty()) {
            Some(r) => format!("{r} {irt}"),
            None => irt.clone(),
        };
        b = b.references(refs);
    }

    let alt = MultiPart::alternative_plain_html(d.body.clone(), text_to_html(&d.body));
    let mut files: Vec<(String, String, Vec<u8>)> = Vec::new();
    for path in &d.attachments {
        let bytes = std::fs::read(path).map_err(|e| format!("Cannot read attachment {path}: {e}"))?;
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "attachment".into());
        files.push((name, guess_mime(path).to_string(), bytes));
    }
    files.extend(forwarded);

    let msg = if files.is_empty() {
        b.multipart(alt)
    } else {
        let mut mixed = MultiPart::mixed().multipart(alt);
        for (name, mime, bytes) in files {
            let ct = ContentType::parse(&mime).unwrap_or(ContentType::parse("application/octet-stream").unwrap());
            let part: SinglePart = Attachment::new(name).body(bytes, ct);
            mixed = mixed.singlepart(part);
        }
        b.multipart(mixed)
    };
    msg.map_err(|e| format!("Cannot build message: {e}"))
}

pub fn send(acct: &Account, auth: &Auth, msg: &Message) -> Result<(), String> {
    let host = acct.smtp_host.as_str();
    let builder = match acct.smtp_security.as_str() {
        "ssl" => SmtpTransport::relay(host),
        "starttls" => SmtpTransport::starttls_relay(host),
        _ => Ok(SmtpTransport::builder_dangerous(host)),
    }
    .map_err(|e| format!("SMTP setup failed: {e}"))?
    .port(acct.smtp_port)
    .timeout(Some(Duration::from_secs(60)));

    let transport = match auth {
        Auth::Password(p) => builder
            .credentials(Credentials::new(acct.username.clone(), p.clone()))
            .authentication(vec![Mechanism::Plain, Mechanism::Login]),
        Auth::Bearer(t) => builder
            .credentials(Credentials::new(acct.username.clone(), t.clone()))
            .authentication(vec![Mechanism::Xoauth2]),
    }
    .build();

    transport.send(msg).map(|_| ()).map_err(|e| {
        let mut m = format!("Sending failed: {e}");
        if acct.kind == "m365" {
            m.push_str("\nIf this says authentication is disabled, enable 'Authenticated SMTP' for this mailbox in the Microsoft 365 admin center.");
        }
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct() -> Account {
        Account { id: 1, name: "Test User".into(), email: "me@example.com".into(), kind: "imap".into(), imap_host: "".into(), imap_port: 993,
            smtp_host: "".into(), smtp_port: 465, smtp_security: "ssl".into(), username: "me".into(), tenant: "".into(),
            client_id: "".into(), client_secret: "".into(), save_sent: true }
    }

    #[test]
    fn splits_addresses() {
        assert_eq!(split_addresses("a@x.com, \"Doe, John\" <j@x.com>; b@y.com"), vec!["a@x.com", "\"Doe, John\" <j@x.com>", "b@y.com"]);
    }

    #[test]
    fn builds_reply_with_attachment() {
        let d = Draft {
            account_id: 1, to: "Alice <alice@example.com>".into(), cc: "".into(), bcc: "hidden@example.com".into(),
            subject: "Re: Café".into(), body: "Thanks!\n\n> original line".into(),
            in_reply_to: Some("<orig@x>".into()), references: Some("<r0@x>".into()),
            attachments: vec![], forward_from: None, reply_to_msg: None, draft: None, local_draft_id: None, message_id: None, link_large: false,
        };
        let m = build(&acct(), &d, vec![("a.pdf".into(), "application/pdf".into(), b"%PDF-1.4".to_vec())]).unwrap();
        let s = String::from_utf8_lossy(&m.formatted()).to_string();
        assert!(s.contains("In-Reply-To: <orig@x>"));
        assert!(s.contains("References: <r0@x> <orig@x>"));
        assert!(s.contains("multipart/mixed"));
        assert!(s.contains("filename=\"a.pdf\""));
        assert!(!s.contains("hidden@example.com"), "Bcc must not appear in headers");
        assert_eq!(m.envelope().to().len(), 2, "Bcc must be in the envelope");
    }

    #[test]
    fn rejects_no_recipients() {
        let d = Draft { account_id: 1, to: "".into(), cc: "".into(), bcc: "".into(), subject: "x".into(), body: "y".into(),
            in_reply_to: None, references: None, attachments: vec![], forward_from: None, reply_to_msg: None, draft: None, local_draft_id: None, message_id: None, link_large: false };
        assert!(build(&acct(), &d, vec![]).is_err());
        let dm = build_draft(&acct(), &Draft { to: "sar".into(), ..d }, vec![]).unwrap();
        let s = String::from_utf8_lossy(&dm.formatted()).to_string();
        assert!(s.contains("X-KiteMail-Draft:"), "{s}");
    }
}
