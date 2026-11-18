//! One HTTP client setup for the whole app (OAuth, Nextcloud/DAV, S3, LanguageTool, pairing).
//! Uses the platform TLS (Windows SChannel / OpenSSL) – the same stack as IMAP and SMTP –
//! instead of bundling a second TLS library (rustls + ring), which keeps the app smaller.

use std::sync::Arc;

pub fn agent() -> ureq::AgentBuilder {
    let mut b = ureq::AgentBuilder::new().user_agent(concat!("KiteMail/", env!("CARGO_PKG_VERSION")));
    match native_tls::TlsConnector::new() {
        Ok(tls) => b = b.tls_connector(Arc::new(tls)),
        Err(e) => eprintln!("TLS setup failed: {e}"),
    }
    b
}

#[cfg(test)]
mod tests {
    /// HTTPS through the platform TLS: a trusted local server works, a wrong host name is refused.
    /// Needs local HTTPS servers: :8443 with the trusted test certificate, :8444 self-signed; skipped unless LM_IT=1.
    #[test]
    fn https_uses_platform_tls() {
        if std::env::var("LM_IT").is_err() {
            return;
        }
        let a = super::agent().build();
        let v: serde_json::Value = a.get("https://localhost:8443/ping.json").call().unwrap().into_json().unwrap();
        assert_eq!(v["ok"], true);
        // self-signed certificate nobody trusts (server on :8444) -> must be refused
        assert!(a.get("https://localhost:8444/ping.json").call().is_err());
    }
}
