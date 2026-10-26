//! Build the "Create your Slack app" URL from the committed manifest.

use serde_json::Value;

const MANIFEST: &str = include_str!("../../../assets/slack-app-manifest.json");

pub fn create_slack_app_url(redirect_uri: &str) -> Result<String, String> {
    let mut manifest: Value =
        serde_json::from_str(MANIFEST).map_err(|err| format!("manifest: {err}"))?;
    manifest
        .pointer_mut("/oauth_config/redirect_urls")
        .map(|urls| *urls = serde_json::json!([redirect_uri]))
        .ok_or_else(|| "manifest is missing redirect_urls".to_owned())?;
    let json = serde_json::to_string(&manifest).map_err(|err| err.to_string())?;
    Ok(format!(
        "https://api.slack.com/apps?new_app=1&manifest_json={}",
        form_encode(&json)
    ))
}

fn form_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_uses_the_configured_redirect_and_no_client_id() {
        let url = create_slack_app_url("http://127.0.0.1:47821/mcp/oauth/callback").unwrap();
        assert!(url.starts_with("https://api.slack.com/apps?new_app=1&manifest_json="));
        let json = urlencoding_decode(url.split_once("manifest_json=").unwrap().1);
        assert!(json.contains("http://127.0.0.1:47821/mcp/oauth/callback"));
        assert!(!json.to_ascii_lowercase().contains("client_id"));
        assert!(json.contains("PR-Nexus"));
    }

    fn urlencoding_decode(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'%' if index + 2 < bytes.len() => {
                    let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap();
                    out.push(u8::from_str_radix(hex, 16).unwrap());
                    index += 3;
                }
                other => {
                    out.push(other);
                    index += 1;
                }
            }
        }
        String::from_utf8(out).unwrap()
    }
}
