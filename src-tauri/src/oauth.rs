//! OAuth 2.0 Authorization Code + PKCE flow for desktop apps, using a loopback redirect
//! (http://localhost:<random port>). Works with Microsoft Entra ID (Microsoft 365) and Google.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::{distributions::Alphanumeric, Rng};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

pub struct Provider {
    pub auth_url: String,
    pub token_url: String,
    pub scopes: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub extra_auth_params: Vec<(&'static str, &'static str)>,
    /// Host used in the redirect URI ("localhost" for Microsoft, "127.0.0.1" for Google).
    pub redirect_host: &'static str,
}

#[derive(Deserialize, Debug)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

pub fn provider(kind: &str, tenant: &str, client_id: &str, client_secret: &str) -> Result<Provider, String> {
    if client_id.trim().is_empty() {
        return Err("OAuth Client ID is required (see README for app registration steps)".into());
    }
    match kind {
        "m365" => {
            let tenant = if tenant.trim().is_empty() { "organizations" } else { tenant.trim() };
            Ok(Provider {
                auth_url: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"),
                token_url: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
                scopes: "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send offline_access openid email".into(),
                client_id: client_id.trim().into(),
                client_secret: None,
                extra_auth_params: vec![("prompt", "select_account")],
                redirect_host: "localhost",
            })
        }
        "gmail" => Ok(Provider {
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            scopes: "https://mail.google.com/".into(),
            client_id: client_id.trim().into(),
            client_secret: Some(client_secret.trim().to_string()).filter(|s| !s.is_empty()),
            extra_auth_params: vec![("access_type", "offline"), ("prompt", "consent")],
            redirect_host: "127.0.0.1",
        }),
        _ => Err(format!("Unknown OAuth provider: {kind}")),
    }
}

fn random_string(len: usize) -> String {
    rand::thread_rng().sample_iter(&Alphanumeric).take(len).map(char::from).collect()
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

const DONE_PAGE: &str = "<!doctype html><html><body style=\"font-family:system-ui;text-align:center;padding-top:15vh\">\
<h2>Kite Mail</h2><p>Sign-in complete. You can close this tab and return to Kite Mail.</p></body></html>";

/// Runs the full interactive sign-in. `open_browser` must open the given URL in the system browser.
pub fn authorize(
    p: &Provider,
    login_hint: &str,
    open_browser: impl Fn(&str) -> Result<(), String>,
) -> Result<TokenResponse, String> {
    // Listen on IPv4 loopback with a random port; also try IPv6 on the same port so
    // "localhost" works whichever address the browser picks.
    let v4 = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Cannot open local port: {e}"))?;
    let port = v4.local_addr().map_err(|e| e.to_string())?.port();
    let v6 = TcpListener::bind(("::1", port)).ok();
    v4.set_nonblocking(true).map_err(|e| e.to_string())?;
    if let Some(l) = &v6 {
        let _ = l.set_nonblocking(true);
    }

    let redirect_uri = format!("http://{}:{port}/", p.redirect_host);
    let state = random_string(24);
    let verifier = random_string(64);
    let challenge = pkce_challenge(&verifier);

    let mut url = url::Url::parse(&p.auth_url).map_err(|e| e.to_string())?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", &p.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", &p.scopes)
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        if !login_hint.is_empty() {
            q.append_pair("login_hint", login_hint);
        }
        for (k, v) in &p.extra_auth_params {
            q.append_pair(k, v);
        }
    }
    open_browser(url.as_str())?;

    let deadline = Instant::now() + Duration::from_secs(300);
    let code = loop {
        if Instant::now() > deadline {
            return Err("Sign-in timed out after 5 minutes".into());
        }
        let conn = match v4.accept() {
            Ok((s, _)) => Some(s),
            Err(_) => v6.as_ref().and_then(|l| l.accept().ok()).map(|(s, _)| s),
        };
        let Some(stream) = conn else {
            std::thread::sleep(Duration::from_millis(150));
            continue;
        };
        if let Some(result) = handle_redirect(stream, &state)? {
            break result;
        }
    };

    let mut form: Vec<(&str, &str)> = vec![
        ("client_id", &p.client_id),
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect_uri),
        ("code_verifier", &verifier),
        ("scope", &p.scopes),
    ];
    if let Some(sec) = &p.client_secret {
        form.push(("client_secret", sec));
    }
    post_token(&p.token_url, &form)
}

/// Returns Ok(Some(code)) when the redirect carried a code, Ok(None) for unrelated requests (favicon).
fn handle_redirect(mut stream: TcpStream, expected_state: &str) -> Result<Option<String>, String> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req.split_whitespace().nth(1).unwrap_or("/");
    let parsed = url::Url::parse(&format!("http://localhost{path}")).map_err(|e| e.to_string())?;
    let get = |k: &str| parsed.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());

    let respond = |stream: &mut TcpStream, status: &str, body: &str| {
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.flush();
    };

    if let Some(err) = get("error") {
        let desc = get("error_description").unwrap_or_default();
        respond(&mut stream, "200 OK", &format!("<h3>Sign-in failed</h3><p>{}</p>", html_escape(&desc)));
        return Err(format!("Sign-in failed: {err} {desc}"));
    }
    match get("code") {
        Some(code) => {
            if get("state").as_deref() != Some(expected_state) {
                respond(&mut stream, "400 Bad Request", "State mismatch");
                return Err("Sign-in failed: state mismatch".into());
            }
            respond(&mut stream, "200 OK", DONE_PAGE);
            Ok(Some(code))
        }
        None => {
            respond(&mut stream, "404 Not Found", "");
            Ok(None)
        }
    }
}

pub fn refresh(p: &Provider, refresh_token: &str) -> Result<TokenResponse, String> {
    let mut form: Vec<(&str, &str)> = vec![
        ("client_id", &p.client_id),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("scope", &p.scopes),
    ];
    if let Some(sec) = &p.client_secret {
        form.push(("client_secret", sec));
    }
    post_token(&p.token_url, &form)
}

fn post_token(url: &str, form: &[(&str, &str)]) -> Result<TokenResponse, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build();
    match agent.post(url).send_form(form) {
        Ok(resp) => resp.into_json::<TokenResponse>().map_err(|e| format!("Bad token response: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("error_description").or(v.get("error")).and_then(|x| x.as_str()).map(String::from))
                .unwrap_or(body);
            Err(format!("Token request failed ({code}): {msg}"))
        }
        Err(e) => Err(format!("Token request failed: {e}")),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pkce_rfc7636_vector() {
        // Test vector from RFC 7636 Appendix B
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    /// Full loopback flow: simulated browser hits the redirect URI, mock token endpoint
    /// verifies the PKCE verifier against the challenge from the authorize URL.
    #[test]
    fn loopback_flow_with_mock_provider() {
        use std::sync::mpsc;
        let token_srv = TcpListener::bind("127.0.0.1:0").unwrap();
        let token_url = format!("http://127.0.0.1:{}/token", token_srv.local_addr().unwrap().port());
        let (chal_tx, chal_rx) = mpsc::channel::<String>();

        let server = std::thread::spawn(move || {
            let (mut s, _) = token_srv.accept().unwrap();
            let mut buf = vec![0u8; 16384];
            let mut n = 0;
            loop {
                n += s.read(&mut buf[n..]).unwrap();
                let txt = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(i) = txt.find("\r\n\r\n") {
                    let len: usize = txt.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap())).unwrap_or(0);
                    if n >= i + 4 + len { break; }
                }
            }
            let txt = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = txt.split("\r\n\r\n").nth(1).unwrap();
            let form: std::collections::HashMap<String, String> = url::form_urlencoded::parse(body.as_bytes()).into_owned().collect();
            let challenge = chal_rx.recv().unwrap();
            let ok = form["code"] == "THE_CODE" && pkce_challenge(&form["code_verifier"]) == challenge
                && form["grant_type"] == "authorization_code" && form.get("client_secret").map(|s| s.as_str()) == Some("sec");
            let json = if ok { r#"{"access_token":"AT","refresh_token":"RT","expires_in":3599}"# } else { r#"{"error":"invalid_grant"}"# };
            let status = if ok { "200 OK" } else { "400 Bad Request" };
            write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}", json.len()).unwrap();
        });

        let p = Provider {
            auth_url: "https://login.example.com/authorize".into(),
            token_url,
            scopes: "mail".into(),
            client_id: "cid".into(),
            client_secret: Some("sec".into()),
            extra_auth_params: vec![],
            redirect_host: "127.0.0.1",
        };
        let browser = move |auth: &str| -> Result<(), String> {
            let u = url::Url::parse(auth).unwrap();
            let q: std::collections::HashMap<String, String> = u.query_pairs().into_owned().collect();
            assert_eq!(q["code_challenge_method"], "S256");
            assert_eq!(q["login_hint"], "me@x.com");
            chal_tx.send(q["code_challenge"].clone()).unwrap();
            let redirect = q["redirect_uri"].clone();
            let state = q["state"].clone();
            std::thread::spawn(move || {
                // A stray favicon request first, then the real redirect.
                let host = redirect.trim_start_matches("http://").trim_end_matches('/').to_string();
                let mut s = TcpStream::connect(&host).unwrap();
                write!(s, "GET /favicon.ico HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
                let mut sink = String::new();
                let _ = s.read_to_string(&mut sink);
                let mut s = TcpStream::connect(&host).unwrap();
                write!(s, "GET /?code=THE_CODE&state={state} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
                let mut page = String::new();
                let _ = s.read_to_string(&mut page);
                assert!(page.contains("Sign-in complete"));
            });
            Ok(())
        };
        let t = authorize(&p, "me@x.com", browser).unwrap();
        server.join().unwrap();
        assert_eq!(t.access_token, "AT");
        assert_eq!(t.refresh_token.as_deref(), Some("RT"));
        assert_eq!(t.expires_in, Some(3599));
    }

    #[test]
    fn m365_defaults() {
        let p = provider("m365", "", "abc", "").unwrap();
        assert!(p.auth_url.contains("/organizations/"));
        assert!(p.scopes.contains("IMAP.AccessAsUser.All") && p.scopes.contains("offline_access"));
        assert!(provider("gmail", "", "", "").is_err());
    }
}
