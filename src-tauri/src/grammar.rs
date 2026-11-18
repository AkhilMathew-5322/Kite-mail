//! Grammar & style checking via a LanguageTool server (public API or self-hosted / Nextcloud).

use crate::models::GrammarIssue;
use std::time::Duration;

pub fn check(base_url: &str, language: &str, text: &str) -> Result<Vec<GrammarIssue>, String> {
    if text.trim().is_empty() {
        return Ok(vec![]);
    }
    let url = format!("{}/v2/check", base_url.trim_end_matches('/'));
    let lang = if language.trim().is_empty() { "auto" } else { language.trim() };
    let agent = crate::net::agent().timeout(Duration::from_secs(25)).build();
    let resp = agent
        .post(&url)
        .send_form(&[("text", text), ("language", lang), ("level", "default")])
        .map_err(|e| match e {
            ureq::Error::Status(429, _) => "LanguageTool rate limit reached - wait a minute or use your own server".to_string(),
            ureq::Error::Status(code, r) => format!("LanguageTool error {code}: {}", r.into_string().unwrap_or_default()),
            other => format!("Cannot reach LanguageTool at {base_url}: {other}"),
        })?;
    let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    Ok(parse(&v))
}

/// LanguageTool returns offsets in UTF-16 code units (Java strings), which is exactly what
/// JavaScript string indexes use, so they are passed through unchanged.
pub fn parse(v: &serde_json::Value) -> Vec<GrammarIssue> {
    v["matches"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|m| GrammarIssue {
                    offset: m["offset"].as_u64().unwrap_or(0) as usize,
                    length: m["length"].as_u64().unwrap_or(0) as usize,
                    message: m["message"].as_str().unwrap_or("").to_string(),
                    replacements: m["replacements"]
                        .as_array()
                        .map(|r| r.iter().take(5).filter_map(|x| x["value"].as_str().map(String::from)).collect())
                        .unwrap_or_default(),
                    category: m["rule"]["category"]["name"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_languagetool_response() {
        let v = serde_json::json!({"matches":[{"message":"Possible spelling mistake found.","offset":7,"length":4,
            "replacements":[{"value":"this"},{"value":"thus"}],"rule":{"category":{"name":"Possible Typo"}}}]});
        let r = super::parse(&v);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].offset, r[0].length), (7, 4));
        assert_eq!(r[0].replacements, vec!["this", "thus"]);
        assert_eq!(r[0].category, "Possible Typo");
    }
}
