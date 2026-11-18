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

/// HTML part: the plain body, with the plain-text signature swapped for the HTML signature.
pub fn body_html(body: &str, sig_text: &str, sig_html: &str) -> String {
    let sig_text = sig_text.trim();
    if sig_html.trim().is_empty() || sig_text.is_empty() {
        return text_to_html(body);
    }
    match body.find(sig_text) {
        Some(i) => {
            let clean = strip_active(sig_html);
            let before = text_to_html(&body[..i]);
            let after = text_to_html(&body[i + sig_text.len()..]);
            format!("{}<div class=\"kite-signature\">{clean}</div>{}", before.trim_end_matches("</div>"), after.trim_start_matches("<div style=\"font-family:Segoe UI,Arial,sans-serif;font-size:14px\">"))
        }
        None => text_to_html(body), // signature was edited away: send what the user typed
    }
}

/// Removes scripts and event handlers from user-supplied HTML (signatures).
fn strip_active(html: &str) -> String {
    let mut out = html.to_string();
    for tag in ["script", "iframe", "object", "embed"] {
        while let Some(i) = out.to_ascii_lowercase().find(&format!("<{tag}")) {
            let lower = out.to_ascii_lowercase();
            let end = lower[i..].find(&format!("</{tag}>")).map(|e| i + e + tag.len() + 3).or_else(|| lower[i..].find('>').map(|e| i + e + 1)).unwrap_or(out.len());
            out.replace_range(i..end, "");
        }
    }
    // on*="..." attributes
    let mut res = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(i) = rest.to_ascii_lowercase().find(" on") {
        let after = &rest[i + 3..];
        let name_len = after.chars().take_while(|c| c.is_ascii_alphabetic()).count();
        let tail = &after[name_len..];
        if name_len > 0 && tail.trim_start().starts_with('=') {
            res.push_str(&rest[..i]);
            let v = tail.trim_start()[1..].trim_start();
            let skip = match v.chars().next() {
                Some(q @ ('"' | '\'')) => v[1..].find(q).map(|e| e + 2).unwrap_or(v.len()),
                _ => v.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(v.len()),
            };
            rest = &v[skip..];
        } else {
            res.push_str(&rest[..i + 3]);
            rest = after;
        }
    }
    res.push_str(rest);
    res.replace("javascript:", "")
}

fn own_vcard(name: &str, email: &str, org: &str) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace(',', "\\,").replace(';', "\\;").replace('\n', " ");
    let mut v = format!("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:{}\r\nN:{};;;;\r\nEMAIL;TYPE=INTERNET:{}\r\n", esc(name), esc(name), esc(email));
    if !org.trim().is_empty() {
        v.push_str(&format!("ORG:{}\r\n", esc(org.trim())));
    }
    v.push_str("END:VCARD\r\n");
    v
}

/// Read receipt (RFC 8098 MDN) for a message that asked for one.
pub fn build_receipt(from: &Mailbox, to: &str, subject: &str, original_id: &str) -> Result<(lettre::address::Envelope, Vec<u8>), String> {
    let to_mb: Mailbox = to.trim().parse().map_err(|_| format!("Can't send a receipt to \"{to}\""))?;
    let boundary = format!("kite-mdn-{:016x}", rand::random::<u64>());
    let date = chrono::Utc::now().to_rfc2822();
    let mid = format!("<{:016x}.mdn@kitemail>", rand::random::<u64>());
    let subj = format!("Read: {}", subject.replace(['\r', '\n'], " "));
    let enc_subj = if subj.is_ascii() { subj.clone() } else {
        use base64::Engine;
        format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(&subj))
    };
    let raw = format!(
        "From: {from}\r\nTo: {to_mb}\r\nSubject: {enc_subj}\r\nDate: {date}\r\nMessage-ID: {mid}\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/report; report-type=disposition-notification; boundary=\"{boundary}\"\r\n\r\n\
         --{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n\
         This is a read receipt for the message you sent to {addr}.\r\nSubject: {subject}\r\n\r\n\
         The message was displayed. This does not guarantee that it was read or understood.\r\n\r\n\
         --{boundary}\r\nContent-Type: message/disposition-notification\r\n\r\n\
         Reporting-UA: Kite Mail\r\nFinal-Recipient: rfc822;{addr}\r\nOriginal-Message-ID: {original_id}\r\n\
         Disposition: manual-action/MDN-sent-manually; displayed\r\n\r\n--{boundary}--\r\n",
        addr = from.email,
        subject = subject.replace(['\r', '\n'], " "),
    );
    let env = lettre::address::Envelope::new(Some(from.email.clone()), vec![to_mb.email]).map_err(|e| e.to_string())?;
    Ok((env, raw.into_bytes()))
}

pub fn guess_mime(path: &str) -> &'static str {
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
    let (from_name, from_email) = if d.from_email.trim().is_empty() {
        (acct.name.clone(), acct.email.clone())
    } else {
        (if d.from_name.trim().is_empty() { acct.name.clone() } else { d.from_name.trim().to_string() }, d.from_email.trim().to_string())
    };
    let from_addr: lettre::Address = from_email.parse().map_err(|e| format!("Bad sender address \"{from_email}\": {e}"))?;
    let from = Mailbox::new(Some(from_name.clone()).filter(|n| !n.is_empty()), from_addr.clone());
    let lenient = |s: &str| -> Vec<Mailbox> { split_addresses(s).iter().filter_map(|a| a.parse().ok()).collect() };
    let (to, cc, bcc) = if draft {
        (lenient(&d.to), lenient(&d.cc), lenient(&d.bcc))
    } else {
        (parse_list(&d.to)?, parse_list(&d.cc)?, parse_list(&d.bcc)?)
    };
    if !draft && to.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err("Add at least one recipient".into());
    }

    let mut b = Message::builder().from(from.clone()).subject(d.subject.clone()).user_agent("Kite Mail".into());
    if !d.reply_to.trim().is_empty() {
        let rt: Mailbox = d.reply_to.trim().parse().map_err(|_| format!("\"{}\" is not a valid reply-to address", d.reply_to.trim()))?;
        b = b.reply_to(rt);
    }
    let hdr = |name: &'static str, v: String| lettre::message::header::HeaderValue::new(lettre::message::header::HeaderName::new_from_ascii_str(name), v);
    if !d.organization.trim().is_empty() {
        b = b.raw_header(hdr("Organization", d.organization.trim().to_string()));
    }
    if d.request_receipt {
        b = b.raw_header(hdr("Disposition-Notification-To", from.to_string()));
    }
    b = match d.message_id.as_ref().filter(|m| !m.is_empty()) {
        Some(m) => b.message_id(Some(m.clone())),
        None => b.message_id(None),
    };
    if draft {
        let raw = serde_json::json!({ "to": d.to, "cc": d.cc, "bcc": d.bcc, "receipt": d.request_receipt }).to_string();
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

    let alt = MultiPart::alternative_plain_html(d.body.clone(), body_html(&d.body, &d.sig_text, &d.sig_html));
    let mut files: Vec<(String, String, Vec<u8>)> = Vec::new();
    for path in &d.attachments {
        let bytes = std::fs::read(path).map_err(|e| format!("Cannot read attachment {path}: {e}"))?;
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "attachment".into());
        files.push((name, guess_mime(path).to_string(), bytes));
    }
    files.extend(forwarded);
    if d.vcard {
        let file = format!("{}.vcf", if from_name.trim().is_empty() { "contact" } else { from_name.trim() });
        files.push((file, "text/vcard".into(), own_vcard(&from_name, &from_email, &d.organization).into_bytes()));
    }

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
    transport(acct, auth)?.send(msg).map(|_| ()).map_err(|e| smtp_error(acct, e))
}

fn transport(acct: &Account, auth: &Auth) -> Result<SmtpTransport, String> {
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
    Ok(transport)
}

/// Sends pre-built bytes (read receipts).
pub fn send_raw(acct: &Account, auth: &Auth, env: &lettre::address::Envelope, bytes: &[u8]) -> Result<(), String> {
    transport(acct, auth)?.send_raw(env, bytes).map(|_| ()).map_err(|e| smtp_error(acct, e))
}

fn smtp_error(acct: &Account, e: lettre::transport::smtp::Error) -> String {
    {
        let mut m = format!("Sending failed: {e}");
        if acct.kind == "m365" {
            m.push_str("\nIf this says authentication is disabled, enable 'Authenticated SMTP' for this mailbox in the Microsoft 365 admin center.");
        }
        m
    }
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
    fn identity_signature_vcard_receipt() {
        let mut d: Draft = serde_json::from_value(serde_json::json!({
            "account_id": 1, "to": "x@example.org", "subject": "Hi",
            "body": "Hello\n\n-- \nAkhil Mathew\nTLR\n\nOn Mon, X wrote:\n> old",
            "from_name": "Legal Desk", "from_email": "legal@example.com", "reply_to": "cases@example.com",
            "organization": "The Law Reporters", "request_receipt": true, "vcard": true,
            "sig_text": "Akhil Mathew\nTLR",
            "sig_html": "<b>Akhil Mathew</b><img src=x onerror=\"alert(1)\"><script>bad()</script>"
        })).unwrap();
        let raw = String::from_utf8(build(&acct(), &d, vec![]).unwrap().formatted()).unwrap();
        assert!(raw.contains("From: \"Legal Desk\" <legal@example.com>") || raw.contains("From: Legal Desk <legal@example.com>"), "{raw}");
        assert!(raw.contains("Reply-To: cases@example.com"));
        assert!(raw.contains("Organization: The Law Reporters"));
        assert!(raw.contains("Disposition-Notification-To:"));
        assert!(raw.contains("Legal Desk.vcf") && raw.contains("BEGIN:VCARD"));
        let html = body_html(&d.body, &d.sig_text, &d.sig_html);
        assert!(html.contains("<b>Akhil Mathew</b>") && !html.contains("onerror") && !html.contains("bad()"), "{html}");
        assert!(html.find("kite-signature").unwrap() < html.find("old").unwrap());
        assert_eq!(html.matches("<div").count(), html.matches("</div>").count());
        // signature edited away in the editor -> plain conversion, no HTML signature
        d.body = "Hello".into();
        assert!(!body_html(&d.body, &d.sig_text, &d.sig_html).contains("kite-signature"));
        // read receipt
        let me: Mailbox = "Akhil <me@example.com>".parse().unwrap();
        let (env, bytes) = build_receipt(&me, "Sender <s@example.org>", "Contract", "<abc@x>").unwrap();
        let txt = String::from_utf8(bytes).unwrap();
        assert!(txt.contains("report-type=disposition-notification") && txt.contains("Original-Message-ID: <abc@x>") && txt.contains("Subject: Read: Contract"));
        assert_eq!(env.to()[0].to_string(), "s@example.org");
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
            ..Default::default()
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
            in_reply_to: None, references: None, attachments: vec![], forward_from: None, reply_to_msg: None, draft: None, local_draft_id: None, message_id: None, link_large: false, ..Default::default() };
        assert!(build(&acct(), &d, vec![]).is_err());
        let dm = build_draft(&acct(), &Draft { to: "sar".into(), ..d }, vec![]).unwrap();
        let s = String::from_utf8_lossy(&dm.formatted()).to_string();
        assert!(s.contains("X-KiteMail-Draft:"), "{s}");
    }
}
