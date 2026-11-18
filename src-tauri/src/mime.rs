//! Message parsing helpers: header summaries, message views, IMAP folder-name decoding.

use crate::models::{AttachmentInfo, DraftView, MessageView, MsgSummary};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use mail_parser::{Address, HeaderValue, MessageParser, MimeHeaders};

pub struct HeaderInfo {
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub date: i64,
    pub has_attach: bool,
}

pub fn format_address_list(a: Option<&Address>) -> String {
    let Some(a) = a else { return String::new() };
    a.iter()
        .map(|x| match (x.name(), x.address()) {
            (Some(n), Some(ad)) if !n.trim().is_empty() => format!("{} <{}>", n.trim(), ad),
            (_, Some(ad)) => ad.to_string(),
            (Some(n), None) => n.to_string(),
            _ => String::new(),
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn text_list(v: &HeaderValue) -> String {
    match v {
        HeaderValue::Text(t) => t.to_string(),
        HeaderValue::TextList(l) => l.iter().map(|s| format!("<{s}>")).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
    .split_whitespace()
    .map(|s| if s.starts_with('<') { s.to_string() } else { format!("<{s}>") })
    .collect::<Vec<_>>()
    .join(" ")
}

/// Parse a header block (BODY[HEADER]) into list-view fields.
pub fn parse_headers(raw: &[u8]) -> HeaderInfo {
    let parser = MessageParser::default();
    let Some(m) = parser.parse(raw) else {
        return HeaderInfo { subject: String::new(), from_name: String::new(), from_addr: String::new(), to: String::new(), date: 0, has_attach: false };
    };
    let (from_name, from_addr) = m
        .from()
        .and_then(|a| a.first())
        .map(|a| (a.name().unwrap_or("").to_string(), a.address().unwrap_or("").to_string()))
        .unwrap_or_default();
    let has_attach = m
        .content_type()
        .map(|ct| ct.ctype().eq_ignore_ascii_case("multipart") && ct.subtype().map(|s| s.eq_ignore_ascii_case("mixed")).unwrap_or(false))
        .unwrap_or(false);
    HeaderInfo {
        subject: m.subject().unwrap_or("").to_string(),
        from_name,
        from_addr,
        to: format_address_list(m.to()),
        date: m.date().map(|d| d.to_timestamp()).unwrap_or(0),
        has_attach,
    }
}

/// Short one-line preview of a message body (skips quoted reply lines).
pub fn preview(text: &str) -> String {
    let joined: String = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('>'))
        .take(12)
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = joined.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect();
    if out.chars().count() == 160 {
        out.push('…');
    }
    out
}

/// Plain-text body used for local search and previews. HTML-only messages are converted with
/// block-level line breaks so words from different paragraphs/cells don't run together.
pub fn search_text(raw: &[u8]) -> String {
    let Some(m) = MessageParser::default().parse(raw) else { return String::new() };
    let text = match m.text_part(0).map(|p| &p.body) {
        Some(mail_parser::PartType::Text(t)) => t.to_string(),
        Some(mail_parser::PartType::Html(h)) => html_to_text(h),
        _ => m.body_text(0).map(|t| t.into_owned()).unwrap_or_default(),
    };
    text.chars().take(20_000).collect()
}

/// Minimal HTML -> text: drops <head>/<style>/<script>, turns block tags into line breaks,
/// strips remaining tags and decodes common entities.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let end = match html[i..].find('>') { Some(e) => i + e, None => break };
            let tag = &lower[i + 1..end];
            let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if !tag.starts_with('/') && matches!(name.as_str(), "style" | "script" | "head" | "title") {
                let close = format!("</{name}");
                i = lower[end..].find(&close).map(|p| end + p).unwrap_or(bytes.len());
                if let Some(e) = html[i..].find('>') { i += e + 1; }
                continue;
            }
            match name.as_str() {
                "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "table" | "blockquote" | "hr" | "ul" | "ol" => out.push('\n'),
                "td" | "th" => out.push(' '),
                _ => {}
            }
            i = end + 1;
        } else {
            let next = html[i..].find('<').map(|p| i + p).unwrap_or(bytes.len());
            out.push_str(&html[i..next]);
            i = next;
        }
    }
    let decoded = out
        .replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
        .replace("&quot;", "\"").replace("&#39;", "'").replace("&rsquo;", "’").replace("&ndash;", "–").replace("&mdash;", "—");
    decoded.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n")
}

fn attachment_name(part: &mail_parser::MessagePart, idx: usize) -> String {
    if let Some(n) = part.attachment_name() {
        return n.to_string();
    }
    if part.is_content_type("message", "rfc822") {
        return format!("attached-message-{}.eml", idx + 1);
    }
    let ext = part.content_type().and_then(|c| c.subtype()).unwrap_or("bin");
    format!("attachment-{}.{}", idx + 1, ext)
}

fn mime_of(part: &mail_parser::MessagePart) -> String {
    part.content_type()
        .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")).to_lowercase())
        .unwrap_or_else(|| "application/octet-stream".into())
}

pub fn build_view(raw: &[u8], summary: MsgSummary) -> Result<MessageView, String> {
    let m = MessageParser::default().parse(raw).ok_or("Could not parse message")?;
    let mut html = m.body_html(0).map(|s| s.into_owned()).unwrap_or_default();
    let text = m.body_text(0).map(|s| s.into_owned()).unwrap_or_default();

    let mut attachments = Vec::new();
    for (idx, part) in m.attachments().enumerate() {
        let mime = mime_of(part);
        let mut inline = false;
        if let Some(cid) = part.content_id() {
            let cid = cid.trim_matches(|c| c == '<' || c == '>');
            let needle = format!("cid:{cid}");
            if mime.starts_with("image/") && html.contains(&needle) {
                let data = format!("data:{};base64,{}", mime, STANDARD.encode(part.contents()));
                html = html.replace(&needle, &data);
                inline = true;
            }
        }
        attachments.push(AttachmentInfo {
            index: idx,
            name: attachment_name(part, idx),
            mime,
            size: part.contents().len(),
            inline,
        });
    }

    let lower = html.to_ascii_lowercase();
    let has_remote = lower.contains("src=\"http") || lower.contains("src='http") || lower.contains("url(http") || lower.contains("src=http");

    let mut references = text_list(m.references());
    let message_id = m.message_id().map(|s| format!("<{s}>")).unwrap_or_default();
    if references.is_empty() {
        references = text_list(m.in_reply_to());
    }

    Ok(MessageView {
        to: format_address_list(m.to()),
        cc: format_address_list(m.cc()),
        reply_to: format_address_list(m.reply_to()),
        message_id,
        references,
        html,
        text,
        has_remote,
        attachments,
        mdn_to: m.header_raw("Disposition-Notification-To").map(|v| v.trim().to_string()).unwrap_or_default(),
        summary,
    })
}

/// Returns (file name, mime type, bytes) for attachment `index`.
pub fn attachment(raw: &[u8], index: usize) -> Result<(String, String, Vec<u8>), String> {
    let m = MessageParser::default().parse(raw).ok_or("Could not parse message")?;
    let part = m.attachments().nth(index).ok_or("Attachment not found")?;
    Ok((attachment_name(part, index), mime_of(part), part.contents().to_vec()))
}

/// Re-opens a saved draft for editing.
pub fn draft_view(account_id: i64, raw: &[u8]) -> Result<DraftView, String> {
    let m = MessageParser::default().parse(raw).ok_or("Could not parse draft")?;
    let mut to = format_address_list(m.to());
    let mut cc = format_address_list(m.cc());
    let mut bcc = format_address_list(m.bcc());
    let from_email = m.from().and_then(|a| a.first()).and_then(|a| a.address()).unwrap_or("").to_string();
    if let Some(enc) = m.header_raw(crate::smtp::DRAFT_HEADER) {
        if let Ok(bytes) = STANDARD.decode(enc.split_whitespace().collect::<String>()) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let get = |k: &str| v[k].as_str().unwrap_or("").to_string();
                to = get("to");
                cc = get("cc");
                bcc = get("bcc");
            }
        }
    }
    Ok(DraftView {
        account_id,
        from_email,
        receipt: m.header_raw("Disposition-Notification-To").is_some(),
        to,
        cc,
        bcc,
        subject: m.subject().unwrap_or("").to_string(),
        body: m.body_text(0).map(|t| t.trim_end().to_string()).unwrap_or_default(),
        in_reply_to: text_list(m.in_reply_to()),
        references: text_list(m.references()),
        message_id: m.message_id().map(|s| format!("<{s}>")).unwrap_or_default(),
        attachments: m.attachments().enumerate().map(|(i, p)| attachment_name(p, i)).collect(),
    })
}

/// All non-inline attachments (used when forwarding).
pub fn all_attachments(raw: &[u8]) -> Vec<(String, String, Vec<u8>)> {
    let Some(m) = MessageParser::default().parse(raw) else { return vec![] };
    m.attachments()
        .enumerate()
        .map(|(i, p)| (attachment_name(p, i), mime_of(p), p.contents().to_vec()))
        .collect()
}

/// Decode IMAP "modified UTF-7" mailbox names (RFC 3501 §5.1.3), e.g. "Entw&APw-rfe" -> "Entwürfe".
pub fn decode_mutf7(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let chunk = &after[..end];
        if chunk.is_empty() {
            out.push('&');
        } else {
            let mut b64 = chunk.replace(',', "/");
            while !b64.len().is_multiple_of(4) {
                b64.push('=');
            }
            match STANDARD.decode(b64) {
                Ok(bytes) => {
                    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
                Err(_) => {
                    out.push('&');
                    out.push_str(chunk);
                    out.push('-');
                }
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_text_keeps_block_breaks() {
        let t = html_to_text("<html><head><style>p{x:1}</style></head><body><h2>Q4 invoices</h2><p>Hi,</p><table><tr><td>A</td><td>18,500</td></tr></table>Tom &amp; Jerry</body></html>");
        assert_eq!(preview(&t), "Q4 invoices Hi, A 18,500 Tom & Jerry");
        assert!(!t.contains("x:1"));
    }

    #[test]
    fn mutf7() {
        assert_eq!(decode_mutf7("INBOX"), "INBOX");
        assert_eq!(decode_mutf7("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode_mutf7("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-"), "Отправленные");
        assert_eq!(decode_mutf7("A&-B"), "A&B");
    }

    const SAMPLE: &[u8] = b"From: =?UTF-8?B?SsO8cmdlbg==?= <j@example.com>\r\n\
To: Alice <alice@example.com>, bob@example.com\r\n\
Subject: =?UTF-8?Q?Caf=C3=A9_report?=\r\n\
Date: Mon, 16 Nov 2026 10:00:00 +0400\r\n\
Message-ID: <abc@example.com>\r\n\
References: <r1@x> <r2@x>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n\
--b1\r\nContent-Type: multipart/related; boundary=\"b2\"\r\n\r\n\
--b2\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Hello <img src=\"cid:logo1\"></p>\r\n\
--b2\r\nContent-Type: image/png\r\nContent-ID: <logo1>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n\
--b2--\r\n\
--b1\r\nContent-Type: application/pdf; name=\"invoice.pdf\"\r\nContent-Disposition: attachment; filename=\"invoice.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=\r\n\
--b1--\r\n";

    fn summary() -> MsgSummary {
        MsgSummary { account_id: 1, folder: "INBOX".into(), uid: 1, subject: String::new(), from_name: String::new(), from_addr: String::new(), to: String::new(), date: 0, size: 0, seen: false, flagged: false, answered: false, has_attach: false, preview: String::new() }
    }

    #[test]
    fn headers() {
        let h = parse_headers(SAMPLE);
        assert_eq!(h.subject, "Café report");
        assert_eq!(h.from_name, "Jürgen");
        assert_eq!(h.from_addr, "j@example.com");
        assert_eq!(h.to, "Alice <alice@example.com>, bob@example.com");
        assert!(h.has_attach);
        assert_eq!(h.date, 1794808800);
    }

    #[test]
    fn view_inlines_cid_and_lists_attachments() {
        let v = build_view(SAMPLE, summary()).unwrap();
        assert!(v.html.contains("data:image/png;base64,"), "{}", v.html);
        assert!(!v.html.contains("cid:logo1"));
        let names: Vec<_> = v.attachments.iter().filter(|a| !a.inline).map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["invoice.pdf"]);
        assert_eq!(v.message_id, "<abc@example.com>");
        assert_eq!(v.references, "<r1@x> <r2@x>");
        let (name, mime, bytes) = attachment(SAMPLE, v.attachments.iter().find(|a| !a.inline).unwrap().index).unwrap();
        assert_eq!((name.as_str(), mime.as_str()), ("invoice.pdf", "application/pdf"));
        assert!(bytes.starts_with(b"%PDF"));
    }
}
