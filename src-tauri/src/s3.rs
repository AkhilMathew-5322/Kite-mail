//! Amazon S3 and S3-compatible object storage (AWS, Oracle Cloud Object Storage, MinIO, Wasabi,
//! Cloudflare R2, Backblaze B2…): mailbox backups with retention and large attachments sent as
//! expiring download links. Requests are signed with AWS Signature V4 (no SDK, a few hundred KB less).

use crate::db::Db;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;

type R<T> = Result<T, String>;

pub const SECRET_KEY: &str = "s3-secret-key";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct S3Config {
    pub enabled: bool,
    /// Empty = AWS (https://s3.<region>.amazonaws.com). Otherwise the provider's S3 endpoint,
    /// e.g. https://<namespace>.compat.objectstorage.me-dubai-1.oraclecloud.com or http://nas:9000
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    /// Folder ("key prefix") inside the bucket.
    pub prefix: String,
    /// https://endpoint/bucket/key instead of https://bucket.endpoint/key (needed for OCI, MinIO).
    pub path_style: bool,
    /// "off" | "daily" | "weekly"
    pub backup_schedule: String,
    pub backup_keep: u32,
    /// Send large attachments as download links stored in the bucket.
    pub links: bool,
    pub link_threshold_mb: u32,
    /// 1-7 days (S3 pre-signed links can't live longer than 7 days).
    pub link_expire_days: u32,
    pub last_backup: HashMap<String, i64>,
    pub last_error: String,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            region: "us-east-1".into(),
            bucket: String::new(),
            access_key: String::new(),
            prefix: "kitemail".into(),
            path_style: false,
            backup_schedule: "weekly".into(),
            backup_keep: 4,
            links: true,
            link_threshold_mb: 10,
            link_expire_days: 7,
            last_backup: HashMap::new(),
            last_error: String::new(),
        }
    }
}

pub fn load(db: &Db) -> S3Config {
    db.get_setting("s3").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(db: &Db, cfg: &S3Config) -> R<()> {
    db.set_setting("s3", &serde_json::to_string(cfg).map_err(|e| e.to_string())?)
}

// ============================================================ signing (AWS SigV4)

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha256_hex(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}

fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const B: usize = 64;
    let mut k = [0u8; B];
    if key.len() > B {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|x| x ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|x| x ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// RFC 3986 encoding as S3 expects it; `/` kept only in paths.
fn uri_enc(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub struct Client {
    scheme: String,
    host: String,
    /// Path before the key: "/bucket" (path style) or "" (virtual host).
    base_path: String,
    region: String,
    access: String,
    secret: String,
    agent: ureq::Agent,
}

/// A request ready to send: URL plus the headers that were signed.
struct Signed {
    url: String,
    headers: Vec<(String, String)>,
}

impl Client {
    pub fn new(cfg: &S3Config, secret: &str) -> R<Self> {
        if cfg.bucket.trim().is_empty() || cfg.access_key.trim().is_empty() || secret.is_empty() {
            return Err("Enter the bucket, access key and secret key".into());
        }
        let region = if cfg.region.trim().is_empty() { "us-east-1".to_string() } else { cfg.region.trim().to_string() };
        let endpoint = if cfg.endpoint.trim().is_empty() {
            format!("https://s3.{region}.amazonaws.com")
        } else if cfg.endpoint.contains("://") {
            cfg.endpoint.trim().trim_end_matches('/').to_string()
        } else {
            format!("https://{}", cfg.endpoint.trim().trim_end_matches('/'))
        };
        let u = url::Url::parse(&endpoint).map_err(|_| format!("\"{endpoint}\" is not a valid endpoint URL"))?;
        let mut host = u.host_str().ok_or("Endpoint has no host name")?.to_string();
        if let Some(p) = u.port() {
            host = format!("{host}:{p}");
        }
        let bucket = cfg.bucket.trim();
        // Buckets with dots break TLS certificates in virtual-host style, so use the path there.
        let path_style = cfg.path_style || bucket.contains('.');
        let (host, base_path) = if path_style { (host, format!("/{}", uri_enc(bucket, false))) } else { (format!("{bucket}.{host}"), String::new()) };
        Ok(Self {
            scheme: u.scheme().to_string(),
            host,
            base_path,
            region,
            access: cfg.access_key.trim().to_string(),
            secret: secret.to_string(),
            agent: ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(120)).build(),
        })
    }

    fn signing_key(&self, date: &str) -> [u8; 32] {
        let k = hmac(format!("AWS4{}", self.secret).as_bytes(), date.as_bytes());
        let k = hmac(&k, self.region.as_bytes());
        let k = hmac(&k, b"s3");
        hmac(&k, b"aws4_request")
    }

    fn canonical_query(q: &[(String, String)]) -> String {
        let mut q: Vec<(String, String)> = q.iter().map(|(k, v)| (uri_enc(k, false), uri_enc(v, false))).collect();
        q.sort();
        q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
    }

    fn path(&self, key: &str) -> String {
        format!("{}/{}", self.base_path, uri_enc(key, true))
    }

    /// Header-signed request (`amz_date` = YYYYMMDDTHHMMSSZ).
    fn sign(&self, method: &str, key: &str, query: &[(String, String)], extra: &[(&str, &str)], payload_hash: &str, amz_date: &str) -> Signed {
        let date = &amz_date[..8];
        let path = self.path(key);
        let mut headers: Vec<(String, String)> = vec![
            ("host".into(), self.host.clone()),
            ("x-amz-content-sha256".into(), payload_hash.into()),
            ("x-amz-date".into(), amz_date.into()),
        ];
        for (k, v) in extra {
            headers.push((k.to_lowercase(), v.trim().to_string()));
        }
        headers.sort();
        let signed_headers = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
        let canon_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
        let cq = Self::canonical_query(query);
        let creq = format!("{method}\n{path}\n{cq}\n{canon_headers}\n{signed_headers}\n{payload_hash}");
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let sts = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha256_hex(creq.as_bytes()));
        let sig = hex(&hmac(&self.signing_key(date), sts.as_bytes()));
        let auth = format!("AWS4-HMAC-SHA256 Credential={}/{scope},SignedHeaders={signed_headers},Signature={sig}", self.access);
        headers.retain(|(k, _)| k != "host");
        headers.push(("Authorization".into(), auth));
        let url = format!("{}://{}{}{}", self.scheme, self.host, path, if cq.is_empty() { String::new() } else { format!("?{cq}") });
        Signed { url, headers }
    }

    /// Pre-signed GET link anyone can open until it expires (max 7 days).
    pub fn presign_get(&self, key: &str, expires_secs: u64, amz_date: &str) -> String {
        let date = &amz_date[..8];
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let query = vec![
            ("X-Amz-Algorithm".to_string(), "AWS4-HMAC-SHA256".to_string()),
            ("X-Amz-Credential".to_string(), format!("{}/{scope}", self.access)),
            ("X-Amz-Date".to_string(), amz_date.to_string()),
            ("X-Amz-Expires".to_string(), expires_secs.clamp(1, 604_800).to_string()),
            ("X-Amz-SignedHeaders".to_string(), "host".to_string()),
        ];
        let path = self.path(key);
        let cq = Self::canonical_query(&query);
        let creq = format!("GET\n{path}\n{cq}\nhost:{}\n\nhost\nUNSIGNED-PAYLOAD", self.host);
        let sts = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha256_hex(creq.as_bytes()));
        let sig = hex(&hmac(&self.signing_key(date), sts.as_bytes()));
        format!("{}://{}{}?{cq}&X-Amz-Signature={sig}", self.scheme, self.host, path)
    }

    fn call(&self, method: &str, key: &str, query: &[(String, String)], what: &str) -> R<ureq::Response> {
        let s = self.sign(method, key, query, &[], EMPTY_SHA256, &now_amz());
        let mut req = self.agent.request(method, &s.url);
        for (k, v) in &s.headers {
            req = req.set(k, v);
        }
        req.call().map_err(|e| s3_error(e, what))
    }

    /// Uploads a file. The body is hashed first (one extra read) so every S3-compatible
    /// provider accepts it, then streamed – large backups never sit in memory.
    pub fn put_file(&self, key: &str, path: &std::path::Path, content_type: &str, disposition: Option<&str>) -> R<()> {
        let mut f = std::fs::File::open(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut len = 0u64;
        loop {
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            len += n as u64;
            h.update(&buf[..n]);
        }
        let hash = hex(&h.finalize());
        let len_s = len.to_string();
        let mut extra = vec![("content-length", len_s.as_str()), ("content-type", content_type)];
        if let Some(d) = disposition {
            extra.push(("content-disposition", d));
        }
        let s = self.sign("PUT", key, &[], &extra, &hash, &now_amz());
        let mut req = self.agent.request("PUT", &s.url);
        for (k, v) in &s.headers {
            req = req.set(k, v);
        }
        let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        req.send(std::io::BufReader::new(f)).map_err(|e| s3_error(e, "Upload"))?;
        Ok(())
    }

    pub fn delete(&self, key: &str) -> R<()> {
        match self.call("DELETE", key, &[], "Delete") {
            Ok(_) => Ok(()),
            Err(e) if e.contains("NoSuchKey") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Keys under `prefix` (all pages).
    pub fn list(&self, prefix: &str) -> R<Vec<String>> {
        let mut keys = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut q = vec![("list-type".to_string(), "2".to_string()), ("prefix".to_string(), prefix.to_string())];
            if let Some(t) = &token {
                q.push(("continuation-token".to_string(), t.clone()));
            }
            let body = self.call("GET", "", &q, "List")?.into_string().map_err(|e| e.to_string())?;
            let doc = roxmltree::Document::parse(&body).map_err(|e| format!("Unexpected reply from storage: {e}"))?;
            let txt = |n: roxmltree::Node, name: &str| n.children().find(|c| c.tag_name().name() == name).and_then(|c| c.text()).map(String::from);
            let root = doc.root_element();
            for c in root.children().filter(|c| c.tag_name().name() == "Contents") {
                if let Some(k) = txt(c, "Key") {
                    keys.push(k);
                }
            }
            token = txt(root, "NextContinuationToken");
            if txt(root, "IsTruncated").as_deref() != Some("true") || token.is_none() {
                break;
            }
        }
        Ok(keys)
    }

    /// Checks that the bucket exists and the keys may list it.
    pub fn test(&self) -> R<()> {
        let q = vec![("list-type".to_string(), "2".to_string()), ("max-keys".to_string(), "1".to_string())];
        self.call("GET", "", &q, "Connection test").map(|_| ())
    }
}

fn now_amz() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// Turns an S3 error reply into something a person can act on.
fn s3_error(e: ureq::Error, what: &str) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let tag = |name: &str| body.split(&format!("<{name}>")).nth(1).and_then(|s| s.split(&format!("</{name}>")).next()).unwrap_or("").to_string();
            let (c, m) = (tag("Code"), tag("Message"));
            let hint = match c.as_str() {
                "SignatureDoesNotMatch" => "the secret key is wrong",
                "InvalidAccessKeyId" => "the access key is wrong",
                "NoSuchBucket" => "the bucket does not exist (check its name and region)",
                "AccessDenied" => "these keys are not allowed to use this bucket",
                "AuthorizationHeaderMalformed" | "IllegalLocationConstraintException" => "the region is wrong",
                "PermanentRedirect" => "the bucket is in another region",
                _ if code == 403 => "access denied",
                _ if code == 404 => "not found",
                _ => "",
            };
            let detail = if !m.is_empty() { m } else if !c.is_empty() { c } else { format!("HTTP {code}") };
            if hint.is_empty() { format!("{what} failed: {detail}") } else { format!("{what} failed – {hint} ({detail})") }
        }
        ureq::Error::Transport(t) => format!("{what} failed: can't reach the storage server ({t})"),
    }
}

pub fn client(db: &Db) -> R<(S3Config, Client)> {
    let cfg = load(db);
    let secret = crate::secrets::get(db, SECRET_KEY).unwrap_or_default();
    let c = Client::new(&cfg, &secret)?;
    Ok((cfg, c))
}

fn join_key(prefix: &str, rest: &str) -> String {
    let p = prefix.trim().trim_matches('/');
    if p.is_empty() { rest.to_string() } else { format!("{p}/{rest}") }
}

// ============================================================ backups

/// Uploads a backup zip to <prefix>/backups/<email>/ and removes the oldest beyond `backup_keep`.
pub fn upload_backup(db: &Db, file: &std::path::Path, email: &str) -> R<String> {
    let (cfg, c) = client(db)?;
    let name = file.file_name().and_then(|n| n.to_str()).ok_or("Bad file name")?;
    let dir = join_key(&cfg.prefix, &format!("backups/{email}/"));
    let key = format!("{dir}{name}");
    c.put_file(&key, file, "application/zip", None)?;
    if cfg.backup_keep > 0 {
        let mut files: Vec<String> = c
            .list(&dir)?
            .into_iter()
            .filter(|k| k.rsplit('/').next().is_some_and(|n| n.starts_with("kitemail-backup-") && n.ends_with(".zip")))
            .collect();
        files.sort();
        let excess = files.len().saturating_sub(cfg.backup_keep as usize);
        for k in files.into_iter().take(excess) {
            c.delete(&k)?;
        }
    }
    Ok(format!("{}/{key}", cfg.bucket))
}

pub fn backup_due(cfg: &S3Config, email: &str) -> bool {
    if !cfg.enabled {
        return false;
    }
    let every = match cfg.backup_schedule.as_str() { "daily" => 86_400, "weekly" => 7 * 86_400, _ => return false };
    chrono::Utc::now().timestamp() - cfg.last_backup.get(email).copied().unwrap_or(0) >= every
}

pub fn mark_backup(db: &Db, email: &str, error: Option<&str>) -> R<()> {
    let mut cfg = load(db);
    match error {
        None => {
            cfg.last_backup.insert(email.to_string(), chrono::Utc::now().timestamp());
            cfg.last_error.clear();
        }
        Some(e) => cfg.last_error = e.to_string(),
    }
    save(db, &cfg)
}

// ============================================================ large attachments as links

/// Uploads a file to <prefix>/attachments/YYYY-MM-DD/<random>/<name> and returns a pre-signed link.
pub fn share_file(db: &Db, path: &std::path::Path) -> R<String> {
    let (cfg, c) = client(db)?;
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("Bad file name")?;
    let rand: u64 = rand::random();
    let key = join_key(&cfg.prefix, &format!("attachments/{}/{rand:016x}/{name}", chrono::Utc::now().format("%Y-%m-%d")));
    let ascii: String = name.chars().map(|ch| if ch.is_ascii_graphic() && ch != '"' && ch != '\\' { ch } else { '_' }).collect();
    let disp = format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", uri_enc(name, false));
    c.put_file(&key, path, crate::smtp::guess_mime(&path.to_string_lossy()), Some(&disp))?;
    Ok(c.presign_get(&key, cfg.link_expire_days.clamp(1, 7) as u64 * 86_400, &now_amz()))
}

/// Replaces local attachments at or above the size limit with S3 download links in the body.
pub fn linkify_attachments(db: &Db, body: &mut String, paths: &mut Vec<String>) -> R<u32> {
    let cfg = load(db);
    let days = cfg.link_expire_days.clamp(1, 7);
    crate::nextcloud::linkify_with(body, paths, cfg.link_threshold_mb, &format!("Files shared via secure download links (expire in {days} days)"), |p| share_file(db, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS's published Signature V4 examples for S3 (examplebucket, 2013-05-24).
    fn aws_example() -> Client {
        let cfg = S3Config { bucket: "examplebucket".into(), access_key: "AKIAIOSFODNN7EXAMPLE".into(), region: "us-east-1".into(), endpoint: "https://s3.amazonaws.com".into(), ..Default::default() };
        Client::new(&cfg, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY").unwrap()
    }

    #[test]
    fn sigv4_get_object_matches_aws_example() {
        let s = aws_example().sign("GET", "test.txt", &[], &[("Range", "bytes=0-9")], EMPTY_SHA256, "20130524T000000Z");
        let auth = &s.headers.iter().find(|(k, _)| k == "Authorization").unwrap().1;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        assert_eq!(s.url, "https://examplebucket.s3.amazonaws.com/test.txt");
    }

    #[test]
    fn sigv4_list_matches_aws_example() {
        let q = vec![("max-keys".to_string(), "2".to_string()), ("prefix".to_string(), "J".to_string())];
        let s = aws_example().sign("GET", "", &q, &[], EMPTY_SHA256, "20130524T000000Z");
        let auth = &s.headers.iter().find(|(k, _)| k == "Authorization").unwrap().1;
        assert!(auth.ends_with("Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"), "{auth}");
    }

    #[test]
    fn presigned_url_matches_aws_example() {
        let url = aws_example().presign_get("test.txt", 86_400, "20130524T000000Z");
        assert!(url.ends_with("X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"), "{url}");
    }

    #[test]
    fn endpoints_and_styles() {
        let mut cfg = S3Config { bucket: "mail".into(), access_key: "a".into(), region: "me-dubai-1".into(), ..Default::default() };
        let c = Client::new(&cfg, "s").unwrap();
        assert_eq!(c.host, "mail.s3.me-dubai-1.amazonaws.com");
        cfg.endpoint = "ns.compat.objectstorage.me-dubai-1.oraclecloud.com".into();
        cfg.path_style = true;
        let c = Client::new(&cfg, "s").unwrap();
        assert_eq!((c.host.as_str(), c.base_path.as_str(), c.scheme.as_str()), ("ns.compat.objectstorage.me-dubai-1.oraclecloud.com", "/mail", "https"));
        cfg.endpoint = "http://nas.local:9000/".into();
        let c = Client::new(&cfg, "s").unwrap();
        assert_eq!(c.path("a b/ü.zip"), "/mail/a%20b/%C3%BC.zip");
        assert_eq!(c.host, "nas.local:9000");
        assert!(Client::new(&S3Config::default(), "s").is_err());
    }
}
