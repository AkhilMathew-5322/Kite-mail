//! Minimal WebDAV / CardDAV / CalDAV client plus vCard 3.0 and iCalendar VTODO mapping.
//! Works with Nextcloud and any standards-compliant DAV server (Radicale, Baïkal, SOGo…).
//! Unknown properties in contacts/tasks are preserved when Kite Mail writes them back.

use crate::models::{Contact, Todo};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::io::Read;
use std::time::Duration;

pub struct Dav {
    auth: String,
    agent: ureq::Agent,
}

#[derive(Debug, Clone)]
pub struct Resource {
    pub href: String,
    pub etag: String,
}

#[derive(Debug, Clone)]
pub struct Collection {
    pub href: String,
    pub name: String,
}

type R<T> = Result<T, String>;

fn err(e: ureq::Error, what: &str) -> String {
    match e {
        ureq::Error::Status(401, _) => "Nextcloud rejected the login – check the username and app password".into(),
        ureq::Error::Status(code, r) => {
            let body = r.into_string().unwrap_or_default();
            let msg = body.split("<s:message>").nth(1).and_then(|s| s.split('<').next()).unwrap_or("").to_string();
            format!("{what} failed (HTTP {code}) {msg}")
        }
        other => format!("{what} failed: {other}"),
    }
}

pub fn etag_clean(e: &str) -> String {
    e.trim().trim_start_matches("W/").to_string()
}

impl Dav {
    pub fn new(user: &str, password: &str) -> Dav {
        Dav {
            auth: format!("Basic {}", STANDARD.encode(format!("{user}:{password}"))),
            agent: ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout(Duration::from_secs(300)).build(),
        }
    }

    fn req(&self, method: &str, url: &str) -> ureq::Request {
        self.agent.request(method, url).set("Authorization", &self.auth).set("User-Agent", "KiteMail/0.3")
    }

    pub fn propfind(&self, url: &str, depth: &str, body: &str) -> R<String> {
        self.req("PROPFIND", url)
            .set("Depth", depth)
            .set("Content-Type", "application/xml; charset=utf-8")
            .send_string(body)
            .map_err(|e| err(e, "PROPFIND"))?
            .into_string()
            .map_err(|e| e.to_string())
    }

    pub fn report(&self, url: &str, depth: &str, body: &str) -> R<String> {
        self.req("REPORT", url)
            .set("Depth", depth)
            .set("Content-Type", "application/xml; charset=utf-8")
            .send_string(body)
            .map_err(|e| err(e, "REPORT"))?
            .into_string()
            .map_err(|e| e.to_string())
    }

    /// Checks credentials against a URL (Depth 0 PROPFIND).
    pub fn ping(&self, url: &str) -> R<()> {
        self.propfind(url, "0", PROPFIND_ETAG).map(|_| ())
    }

    pub fn get(&self, url: &str) -> R<(String, String)> {
        let r = self.req("GET", url).call().map_err(|e| err(e, "GET"))?;
        let etag = etag_clean(r.header("ETag").unwrap_or(""));
        Ok((r.into_string().map_err(|e| e.to_string())?, etag))
    }

    /// PUT text. `if_match`: Some(etag) to update, None to create (If-None-Match: *). Returns the new etag if the server sent one.
    pub fn put(&self, url: &str, content_type: &str, body: &str, if_match: Option<&str>) -> Result<Option<String>, PutError> {
        let mut r = self.req("PUT", url).set("Content-Type", content_type);
        r = match if_match {
            Some(e) if !e.is_empty() => r.set("If-Match", e),
            Some(_) => r,
            None => r.set("If-None-Match", "*"),
        };
        match r.send_string(body) {
            Ok(resp) => Ok(resp.header("ETag").map(etag_clean).filter(|e| !e.is_empty())),
            Err(ureq::Error::Status(412, _)) => Err(PutError::Conflict),
            Err(e) => Err(PutError::Other(err(e, "PUT"))),
        }
    }

    /// Streams a local file to the server.
    pub fn put_file(&self, url: &str, path: &std::path::Path) -> R<()> {
        let f = std::fs::File::open(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        self.req("PUT", url)
            .set("Content-Type", "application/octet-stream")
            .set("Content-Length", &len.to_string())
            .send(std::io::BufReader::new(f))
            .map_err(|e| err(e, "Upload"))?;
        Ok(())
    }

    pub fn delete(&self, url: &str, if_match: Option<&str>) -> R<()> {
        let mut r = self.req("DELETE", url);
        if let Some(e) = if_match.filter(|e| !e.is_empty()) {
            r = r.set("If-Match", e);
        }
        match r.call() {
            Ok(_) | Err(ureq::Error::Status(404, _)) => Ok(()),
            Err(e) => Err(err(e, "DELETE")),
        }
    }

    /// Creates a folder (and its parents). Existing folders are fine.
    pub fn mkcol_all(&self, root: &str, rel_path: &str) -> R<()> {
        let mut url = root.trim_end_matches('/').to_string();
        for part in rel_path.split('/').filter(|p| !p.is_empty()) {
            url = format!("{url}/{}", enc(part));
            match self.req("MKCOL", &url).call() {
                Ok(_) | Err(ureq::Error::Status(405, _)) => {} // 405 = already exists
                Err(e) => return Err(err(e, "Create folder")),
            }
        }
        Ok(())
    }

    /// Lists child resources (non-collections) of a collection with their etags.
    pub fn list(&self, url: &str) -> R<Vec<Resource>> {
        let xml = self.propfind(url, "1", PROPFIND_ETAG)?;
        let self_path = url::Url::parse(url).map(|u| u.path().trim_end_matches('/').to_string()).unwrap_or_default();
        Ok(parse_multistatus(&xml)
            .into_iter()
            .filter(|r| !r.is_collection && r.href.trim_end_matches('/') != self_path)
            .map(|r| Resource { href: r.href, etag: r.etag })
            .collect())
    }

    /// Lists files with their modification time (used for backup retention).
    pub fn list_files(&self, url: &str) -> R<Vec<(String, String)>> {
        let xml = self.propfind(url, "1", PROPFIND_ETAG)?;
        Ok(parse_multistatus(&xml).into_iter().filter(|r| !r.is_collection).map(|r| (r.href, r.name)).collect())
    }

    /// Child collections of a home set, filtered by resource type ("addressbook" or "calendar").
    pub fn collections(&self, home: &str, kind: &str, need_vtodo: bool) -> R<Vec<Collection>> {
        let xml = self.propfind(home, "1", PROPFIND_COLLECTIONS)?;
        Ok(parse_multistatus(&xml)
            .into_iter()
            .filter(|r| r.types.iter().any(|t| t == kind))
            .filter(|r| !need_vtodo || r.components.is_empty() || r.components.iter().any(|c| c == "VTODO"))
            .map(|r| Collection { href: r.href, name: r.name })
            .collect())
    }

    /// Fetches many resources at once (addressbook-multiget / calendar-multiget).
    pub fn multiget(&self, collection_url: &str, hrefs: &[String], caldav: bool) -> R<Vec<(String, String, String)>> {
        let mut out = Vec::new();
        for chunk in hrefs.chunks(100) {
            let items: String = chunk.iter().map(|h| format!("<d:href>{}</d:href>", xml_escape(h))).collect();
            let body = if caldav {
                format!(r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data/></d:prop>{items}</c:calendar-multiget>"#)
            } else {
                format!(r#"<?xml version="1.0" encoding="utf-8"?><card:addressbook-multiget xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav"><d:prop><d:getetag/><card:address-data/></d:prop>{items}</card:addressbook-multiget>"#)
            };
            let xml = self.report(collection_url, "1", &body)?;
            for r in parse_multistatus(&xml) {
                if !r.data.is_empty() {
                    out.push((r.href, r.etag, r.data));
                }
            }
        }
        Ok(out)
    }

    /// Lists VTODO resources of a calendar (calendar-query), skipping events.
    pub fn list_todos(&self, url: &str) -> R<Vec<Resource>> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VTODO"/></c:comp-filter></c:filter></c:calendar-query>"#;
        let xml = self.report(url, "1", body)?;
        Ok(parse_multistatus(&xml).into_iter().filter(|r| !r.is_collection).map(|r| Resource { href: r.href, etag: r.etag }).collect())
    }

    /// Current etag of a single resource.
    pub fn etag_of(&self, url: &str) -> R<String> {
        let xml = self.propfind(url, "0", PROPFIND_ETAG)?;
        Ok(parse_multistatus(&xml).into_iter().next().map(|r| r.etag).unwrap_or_default())
    }

    /// Raw HTTP for OCS (Nextcloud share API).
    pub fn ocs_post(&self, url: &str, form: &[(&str, &str)]) -> R<serde_json::Value> {
        let resp = self
            .req("POST", url)
            .set("OCS-APIRequest", "true")
            .set("Accept", "application/json")
            .send_form(form)
            .map_err(|e| err(e, "Create share link"))?;
        let mut s = String::new();
        resp.into_reader().take(1 << 20).read_to_string(&mut s).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| format!("Unexpected share response: {e}"))
    }
}

#[derive(Debug)]
pub enum PutError {
    Conflict,
    Other(String),
}

const PROPFIND_ETAG: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:getetag/><d:resourcetype/><d:getlastmodified/></d:prop></d:propfind>"#;
const PROPFIND_COLLECTIONS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:resourcetype/><d:displayname/><c:supported-calendar-component-set/></d:prop></d:propfind>"#;

#[derive(Debug, Default)]
pub struct MsEntry {
    pub href: String,
    pub etag: String,
    pub is_collection: bool,
    pub types: Vec<String>,
    pub components: Vec<String>,
    pub name: String,
    pub data: String,
}

/// Parses a 207 multistatus body (namespace-agnostic on local names).
pub fn parse_multistatus(xml: &str) -> Vec<MsEntry> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return vec![] };
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.tag_name().name() == "response") {
        let mut e = MsEntry::default();
        for n in resp.descendants() {
            match n.tag_name().name() {
                "href" if e.href.is_empty() && n.parent().map(|p| p.tag_name().name() == "response").unwrap_or(false) => {
                    e.href = url_decode(n.text().unwrap_or("").trim());
                }
                "getetag" => e.etag = etag_clean(n.text().unwrap_or("")),
                "resourcetype" => {
                    for t in n.children().filter(|c| c.is_element()) {
                        let name = t.tag_name().name().to_string();
                        if name == "collection" {
                            e.is_collection = true;
                        }
                        e.types.push(name);
                    }
                }
                "comp" => {
                    if let Some(v) = n.attribute("name") {
                        e.components.push(v.to_string());
                    }
                }
                "displayname" => e.name = n.text().unwrap_or("").to_string(),
                "getlastmodified" if e.name.is_empty() => e.name = n.text().unwrap_or("").to_string(),
                "address-data" | "calendar-data" => e.data = n.text().unwrap_or("").to_string(),
                _ => {}
            }
        }
        out.push(e);
    }
    out
}

pub fn url_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_string())
}

/// Percent-encodes one path segment.
pub fn enc(seg: &str) -> String {
    url::form_urlencoded::byte_serialize(seg.as_bytes()).collect::<String>().replace('+', "%20")
}

/// Turns a server href (absolute path) into a full URL; also encodes it safely.
pub fn resolve(base: &str, href: &str) -> String {
    let path: String = href.split('/').map(enc).collect::<Vec<_>>().join("/");
    match url::Url::parse(base) {
        Ok(b) => b.join(&path).map(|u| u.to_string()).unwrap_or_else(|_| format!("{base}{path}")),
        Err(_) => format!("{base}{path}"),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// ======================================================================= text formats

/// Unfolds RFC 5545/6350 content lines.
fn unfold(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in raw.replace("\r\n", "\n").split('\n') {
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&line[1..]);
        } else if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

/// Folds a content line at 75 octets without splitting UTF-8 characters.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for ch in line.chars() {
        let n = ch.len_utf8();
        if count + n > 75 {
            out.push_str("\r\n ");
            count = 1;
        }
        out.push(ch);
        count += n;
    }
    out.push_str("\r\n");
    out
}

/// "NAME;PARAM=x:value" -> (NAME upper without group, value)
fn split_prop(line: &str) -> (String, String) {
    let (head, value) = match find_value_colon(line) {
        Some(i) => (&line[..i], &line[i + 1..]),
        None => (line, ""),
    };
    let name = head.split(';').next().unwrap_or("");
    let name = name.rsplit('.').next().unwrap_or(name).to_ascii_uppercase();
    (name, value.to_string())
}

/// The first ':' that is not inside a quoted parameter value.
fn find_value_colon(line: &str) -> Option<usize> {
    let mut q = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => q = !q,
            ':' if !q => return Some(i),
            _ => {}
        }
    }
    None
}

fn params_of(line: &str) -> String {
    let head = match find_value_colon(line) { Some(i) => &line[..i], None => line };
    head.to_ascii_uppercase()
}

pub fn unescape(v: &str) -> String {
    let mut out = String::new();
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace('\n', "\\n").replace(',', "\\,").replace(';', "\\;")
}

// ----------------------------------------------------------------------- vCard

/// Parses a vCard into a Contact + UID.
pub fn vcard_to_contact(raw: &str) -> Option<(Contact, String)> {
    let lines = unfold(raw);
    if !lines.iter().any(|l| l.eq_ignore_ascii_case("BEGIN:VCARD")) {
        return None;
    }
    let mut c = Contact::default();
    let mut uid = String::new();
    let mut n_name = String::new();
    for l in &lines {
        let (name, value) = split_prop(l);
        match name.as_str() {
            "FN" => c.name = unescape(&value),
            "N" => {
                let parts: Vec<String> = value.split(';').map(unescape).collect();
                let given = parts.get(1).cloned().unwrap_or_default();
                let family = parts.first().cloned().unwrap_or_default();
                n_name = format!("{given} {family}").trim().to_string();
            }
            "EMAIL" if c.email.is_empty() => c.email = unescape(&value),
            "TEL" if c.phone.is_empty() => c.phone = unescape(&value).trim_start_matches("tel:").to_string(),
            "ORG" if c.company.is_empty() => c.company = value.split(';').next().map(unescape).unwrap_or_default(),
            "NOTE" => c.notes = unescape(&value),
            "UID" => uid = value.trim().to_string(),
            "X-KITEMAIL-FAVORITE" => c.favorite = value.trim() == "1",
            _ => {}
        }
    }
    if c.name.is_empty() {
        c.name = n_name;
    }
    Some((c, uid))
}

/// Builds a vCard 3.0. Lines from `prev` that Kite Mail does not manage (extra emails/phones,
/// addresses, photos, birthdays…) are kept untouched.
pub fn contact_to_vcard(c: &Contact, uid: &str, prev: Option<&str>) -> String {
    let mut out = String::from("BEGIN:VCARD\r\nVERSION:3.0\r\nPRODID:-//Kite Mail//EN\r\n");
    out += &fold(&format!("UID:{uid}"));
    let fname = if c.name.trim().is_empty() { c.email.clone() } else { c.name.trim().to_string() };
    out += &fold(&format!("FN:{}", escape(&fname)));
    let mut words: Vec<&str> = c.name.split_whitespace().collect();
    let family = if words.len() > 1 { words.pop().unwrap_or("") } else { "" };
    out += &fold(&format!("N:{};{};;;", escape(family), escape(&words.join(" "))));
    if !c.email.is_empty() { out += &fold(&format!("EMAIL;TYPE=INTERNET:{}", c.email.trim())); }
    if !c.phone.is_empty() { out += &fold(&format!("TEL;TYPE=CELL:{}", escape(c.phone.trim()))); }
    if !c.company.is_empty() { out += &fold(&format!("ORG:{}", escape(c.company.trim()))); }
    if !c.notes.is_empty() { out += &fold(&format!("NOTE:{}", escape(&c.notes))); }
    if c.favorite { out += "X-KITEMAIL-FAVORITE:1\r\n"; }
    out += &fold(&format!("REV:{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ")));
    if let Some(p) = prev {
        let (mut skip_email, mut skip_tel) = (true, true);
        for l in unfold(p) {
            let (name, _) = split_prop(&l);
            match name.as_str() {
                "BEGIN" | "END" | "VERSION" | "PRODID" | "UID" | "FN" | "N" | "ORG" | "NOTE" | "REV" | "X-KITEMAIL-FAVORITE" => continue,
                "EMAIL" if skip_email => { skip_email = false; continue; }
                "TEL" if skip_tel => { skip_tel = false; continue; }
                _ => out += &fold(&l),
            }
        }
    }
    out += "END:VCARD\r\n";
    out
}

// ----------------------------------------------------------------------- VTODO

fn parse_ical_time(line: &str, value: &str) -> Option<i64> {
    let v = value.trim();
    let params = params_of(line);
    if params.contains("VALUE=DATE") || v.len() == 8 {
        let d = chrono::NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
        let dt = d.and_hms_opt(9, 0, 0)?; // all-day tasks: remind at 9 am local
        return dt.and_local_timezone(chrono::Local).single().map(|t| t.timestamp());
    }
    if let Some(z) = v.strip_suffix('Z') {
        let dt = chrono::NaiveDateTime::parse_from_str(z, "%Y%m%dT%H%M%S").ok()?;
        return Some(dt.and_utc().timestamp());
    }
    // floating or TZID: treat as this device's local time
    let dt = chrono::NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    dt.and_local_timezone(chrono::Local).single().map(|t| t.timestamp())
}

fn utc_stamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0).unwrap_or_default().format("%Y%m%dT%H%M%SZ").to_string()
}

/// Parses the first VTODO of an iCalendar object.
pub fn ical_to_todo(raw: &str) -> Option<(Todo, String)> {
    let lines = unfold(raw);
    let mut t = Todo::default();
    let mut uid = String::new();
    let (mut in_todo, mut depth_alarm, mut found) = (false, 0, false);
    for l in &lines {
        let up = l.to_ascii_uppercase();
        if up == "BEGIN:VTODO" { in_todo = true; found = true; continue; }
        if up == "END:VTODO" { break; }
        if !in_todo { continue; }
        if up.starts_with("BEGIN:") { depth_alarm += 1; continue; }
        if up.starts_with("END:") { depth_alarm -= 1; continue; }
        if depth_alarm > 0 { continue; }
        let (name, value) = split_prop(l);
        match name.as_str() {
            "UID" => uid = value.trim().to_string(),
            "SUMMARY" => t.title = unescape(&value),
            "DESCRIPTION" => t.notes = unescape(&value),
            "DUE" => t.due = parse_ical_time(l, &value),
            "DTSTART" if t.due.is_none() => t.due = parse_ical_time(l, &value),
            "STATUS" => t.done = value.trim().eq_ignore_ascii_case("COMPLETED"),
            "COMPLETED" => t.done = true,
            "PRIORITY" => t.priority = value.trim().parse::<i64>().map(|p| (1..=4).contains(&p) as i64).unwrap_or(0),
            "CREATED" => t.created = parse_ical_time(l, &value).unwrap_or(0),
            "X-KITEMAIL-REF" => {
                let v = unescape(&value);
                let p: Vec<&str> = v.splitn(3, '|').collect();
                if let (Some(a), Some(f), Some(u)) = (p.first(), p.get(1), p.get(2)) {
                    t.account_id = a.parse().ok();
                    t.folder = Some(f.to_string());
                    t.uid = u.parse().ok();
                }
            }
            _ => {}
        }
    }
    if !found {
        return None;
    }
    if t.title.is_empty() {
        t.title = "(untitled task)".into();
    }
    Some((t, uid))
}

/// Builds a VCALENDAR with one VTODO (with a reminder alarm at the due time). Unknown VTODO
/// properties and the original VTIMEZONE blocks are preserved.
pub fn todo_to_ical(t: &Todo, uid: &str, prev: Option<&str>) -> String {
    let now = utc_stamp(chrono::Utc::now().timestamp());
    let mut cal_extra = String::new(); // e.g. VTIMEZONE blocks
    let mut todo_extra = String::new();
    if let Some(p) = prev {
        let lines = unfold(p);
        let (mut in_todo, mut nested, mut in_tz) = (false, 0, false);
        for l in &lines {
            let up = l.to_ascii_uppercase();
            if up == "BEGIN:VTIMEZONE" { in_tz = true; }
            if in_tz { cal_extra += &fold(l); if up == "END:VTIMEZONE" { in_tz = false; } continue; }
            if up == "BEGIN:VTODO" { in_todo = true; continue; }
            if up == "END:VTODO" { in_todo = false; continue; }
            if !in_todo { continue; }
            if up.starts_with("BEGIN:") { nested += 1; }
            let inside_nested = nested > 0;
            if up.starts_with("END:") { nested -= 1; }
            if inside_nested { continue; } // drop old VALARMs; we write our own
            let (name, _) = split_prop(l);
            if matches!(name.as_str(), "UID" | "SUMMARY" | "DESCRIPTION" | "DUE" | "STATUS" | "COMPLETED" | "PERCENT-COMPLETE" | "PRIORITY" | "DTSTAMP" | "LAST-MODIFIED" | "X-KITEMAIL-REF") {
                continue;
            }
            if name == "DTSTART" && t.due.is_none() { continue; }
            todo_extra += &fold(l);
        }
    }
    let mut out = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Kite Mail//EN\r\n");
    out += &cal_extra;
    out += "BEGIN:VTODO\r\n";
    out += &fold(&format!("UID:{uid}"));
    out += &fold(&format!("DTSTAMP:{now}"));
    out += &fold(&format!("LAST-MODIFIED:{now}"));
    out += &fold(&format!("SUMMARY:{}", escape(&t.title)));
    if !t.notes.is_empty() { out += &fold(&format!("DESCRIPTION:{}", escape(&t.notes))); }
    if let Some(d) = t.due { out += &fold(&format!("DUE:{}", utc_stamp(d))); }
    if t.done {
        out += "STATUS:COMPLETED\r\nPERCENT-COMPLETE:100\r\n";
        out += &fold(&format!("COMPLETED:{now}"));
    } else {
        out += "STATUS:NEEDS-ACTION\r\n";
    }
    if t.priority > 0 { out += "PRIORITY:1\r\n"; }
    if let (Some(a), Some(f), Some(u)) = (t.account_id, &t.folder, t.uid) {
        out += &fold(&format!("X-KITEMAIL-REF:{a}|{}|{u}", escape(f)));
    }
    out += &todo_extra;
    if t.due.is_some() && !t.done {
        out += "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;RELATED=END:PT0S\r\n";
        out += &fold(&format!("DESCRIPTION:{}", escape(&t.title)));
        out += "END:VALARM\r\n";
    }
    out += "END:VTODO\r\nEND:VCALENDAR\r\n";
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcard_roundtrip_keeps_unknown_fields() {
        let phone_made = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:abc-1\r\nFN:Sara Khan\r\nN:Khan;Sara;;;\r\nitem1.EMAIL;TYPE=WORK:sara@kb.com\r\nEMAIL;TYPE=HOME:sara.home@gmail.com\r\nTEL;TYPE=\"CELL,VOICE\":+971 50 1\r\nTEL;TYPE=WORK:+971 4 2\r\nADR;TYPE=WORK:;;DIFC\\, Gate 3;Dubai;;;UAE\r\nNOTE:Prefers\\, calls\\nafter 2pm\r\nBDAY:1990-01-01\r\nEND:VCARD\r\n";
        let (c, uid) = vcard_to_contact(phone_made).unwrap();
        assert_eq!(uid, "abc-1");
        assert_eq!((c.name.as_str(), c.email.as_str(), c.phone.as_str()), ("Sara Khan", "sara@kb.com", "+971 50 1"));
        assert_eq!(c.notes, "Prefers, calls\nafter 2pm");
        let edited = Contact { company: "Kaden Boriss".into(), phone: "+971 55 9".into(), ..c };
        let out = contact_to_vcard(&edited, &uid, Some(phone_made));
        let (back, _) = vcard_to_contact(&out).unwrap();
        assert_eq!(back.phone, "+971 55 9");
        assert_eq!(back.company, "Kaden Boriss");
        assert!(out.contains("sara.home@gmail.com"), "second email kept");
        assert!(out.contains("TEL;TYPE=WORK:+971 4 2"), "second phone kept");
        assert!(out.contains("ADR;TYPE=WORK") && out.contains("BDAY:1990-01-01"));
        assert_eq!(out.matches("FN:").count(), 1);
        assert!(!out.contains("+971 50 1"), "old primary phone replaced");
    }

    #[test]
    fn long_lines_fold_and_unfold() {
        let c = Contact { name: "Ünïcødé ".repeat(20), email: "a@b.c".into(), ..Default::default() };
        let v = contact_to_vcard(&c, "u1", None);
        assert!(v.split("\r\n").all(|l| l.len() <= 75));
        assert_eq!(vcard_to_contact(&v).unwrap().0.name, c.name.trim());
    }

    #[test]
    fn vtodo_roundtrip() {
        let nc = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Nextcloud Tasks\r\nBEGIN:VTIMEZONE\r\nTZID:Asia/Dubai\r\nBEGIN:STANDARD\r\nTZOFFSETFROM:+0400\r\nTZOFFSETTO:+0400\r\nDTSTART:19700101T000000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VTODO\r\nUID:t-9\r\nSUMMARY:Pay invoice\r\nDUE:20261120T050000Z\r\nPRIORITY:1\r\nCATEGORIES:Finance\r\nRELATED-TO:parent-1\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nEND:VALARM\r\nSTATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let (t, uid) = ical_to_todo(nc).unwrap();
        assert_eq!((t.title.as_str(), uid.as_str(), t.priority, t.done), ("Pay invoice", "t-9", 1, false));
        assert_eq!(t.due, Some(1795150800));
        let done = Todo { done: true, notes: "paid; ref 7".into(), ..t };
        let out = todo_to_ical(&done, &uid, Some(nc));
        assert!(out.contains("CATEGORIES:Finance") && out.contains("RELATED-TO:parent-1"), "unknown props kept");
        assert!(out.contains("BEGIN:VTIMEZONE") && out.contains("TZID:Asia/Dubai"));
        assert!(out.contains("STATUS:COMPLETED"));
        assert!(!out.contains("TRIGGER:-PT15M"), "old alarm replaced");
        let (back, _) = ical_to_todo(&out).unwrap();
        assert!(back.done);
        assert_eq!(back.notes, "paid; ref 7");
        assert_eq!(back.due, Some(1795150800));
    }

    #[test]
    fn multistatus_parsing() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav" xmlns:c="urn:ietf:params:xml:ns:caldav">
          <d:response><d:href>/remote.php/dav/addressbooks/users/akhil/contacts/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><card:addressbook/></d:resourcetype><d:displayname>Contacts</d:displayname></d:prop></d:propstat></d:response>
          <d:response><d:href>/remote.php/dav/addressbooks/users/akhil/contacts/Sara%20K.vcf</d:href><d:propstat><d:prop><d:getetag>"e1"</d:getetag><d:resourcetype/></d:prop></d:propstat></d:response>
          <d:response><d:href>/cal/tasks/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set></d:prop></d:propstat></d:response>
        </d:multistatus>"#;
        let r = parse_multistatus(xml);
        assert_eq!(r.len(), 3);
        assert!(r[0].is_collection && r[0].types.contains(&"addressbook".to_string()));
        assert_eq!((r[1].href.as_str(), r[1].etag.as_str()), ("/remote.php/dav/addressbooks/users/akhil/contacts/Sara K.vcf", "\"e1\""));
        assert_eq!(r[2].components, vec!["VTODO"]);
        assert_eq!(resolve("https://nc.example.com/", &r[1].href), "https://nc.example.com/remote.php/dav/addressbooks/users/akhil/contacts/Sara%20K.vcf");
    }
}
