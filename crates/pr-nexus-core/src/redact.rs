//! Log redaction (§9 Logs).
//!
//! Every log line passes through [`redact`] before it reaches a file. It masks
//! GitHub tokens (`gh[pousr]_…`, `github_pat_…`), Slack tokens (`xox[abeposr]-…`
//! and `xoxe-` refresh tokens), OAuth `code`, `code_verifier`, `client_secret`
//! and token parameters (query string and JSON), and `Authorization` headers.

use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::LazyLock;

use regex::Regex;

pub const MASK: &str = "[REDACTED]";

struct Rule {
    pattern: Regex,
    replacement: &'static str,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let rule = |pattern: &str, replacement: &'static str| Rule {
        pattern: Regex::new(pattern).expect("redaction pattern"),
        replacement,
    };
    vec![
        // Headers first so the token inside is masked as one value.
        rule(
            r"(?i)(\bauthorization\b\s*[:=]\s*)((?:bearer|token|basic)\s+)?[^\s,;]+",
            "${1}${2}[REDACTED]",
        ),
        rule(
            r#"(?i)("(?:code|code_verifier|client_secret|access_token|refresh_token)"\s*:\s*")[^"]*""#,
            "${1}[REDACTED]\"",
        ),
        rule(
            r"(?i)(\b(?:code|code_verifier|client_secret|access_token|refresh_token)=)[^&\s]+",
            "${1}[REDACTED]",
        ),
        rule(r"\bgithub_pat_[A-Za-z0-9_]+", "github_pat_[REDACTED]"),
        rule(r"\bgh([pousr])_[A-Za-z0-9]+", "gh${1}_[REDACTED]"),
        rule(
            r"\bxox(e(?:\.xox[a-z])?|[abeposr])-[A-Za-z0-9-]+",
            "xox${1}-[REDACTED]",
        ),
    ]
});

/// Mask every secret in `text`. Borrowed when nothing matched.
pub fn redact(text: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(text);
    for rule in RULES.iter() {
        if rule.pattern.is_match(&out) {
            out = Cow::Owned(
                rule.pattern
                    .replace_all(&out, rule.replacement)
                    .into_owned(),
            );
        }
    }
    out
}

/// A writer that redacts each chunk before passing it on. Log formatters
/// write one whole event per call, so a secret is never split across chunks.
pub struct RedactingWriter<W> {
    inner: W,
}

impl<W> RedactingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.inner.write_all(redact(&text).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_github_and_slack_tokens() {
        let line = "token ghp_abcDEF1234567890 then gho_x1 ghs_y2 ghu_z3 ghr_w4 \
                    github_pat_11ABC_def xoxb-1-2-abc xoxp-123-abc xoxe-1-refresh \
                    xoxe.xoxp-1-rotating xoxa-2-a xapp-not-a-token";
        let out = redact(line);
        for secret in [
            "abcDEF1234567890",
            "x1",
            "y2",
            "z3",
            "w4",
            "11ABC_def",
            "1-2-abc",
            "123-abc",
            "1-refresh",
            "1-rotating",
            "2-a",
        ] {
            assert!(!out.contains(secret), "{secret} leaked: {out}");
        }
        assert!(out.contains("ghp_[REDACTED]"));
        assert!(out.contains("xoxe-[REDACTED]"));
        assert!(out.contains("xapp-not-a-token"));
    }

    #[test]
    fn masks_oauth_parameters_in_queries_and_json() {
        let query = "GET /mcp/oauth/callback?code=abc123&state=s1 \
                     body: grant_type=authorization_code&code_verifier=v3r1f13r&client_secret=sh-h";
        let out = redact(query);
        assert_eq!(
            out,
            "GET /mcp/oauth/callback?code=[REDACTED]&state=s1 \
             body: grant_type=authorization_code&code_verifier=[REDACTED]&client_secret=[REDACTED]"
        );
        let json =
            r#"{"access_token":"t0k","refresh_token":"r3f","client_secret": "s3c","team":"acme"}"#;
        assert_eq!(
            redact(json),
            r#"{"access_token":"[REDACTED]","refresh_token":"[REDACTED]","client_secret": "[REDACTED]","team":"acme"}"#
        );
        // Not a parameter: `error_code=` and `postcode=` stay.
        assert_eq!(
            redact("error_code=42 postcode=1"),
            "error_code=42 postcode=1"
        );
    }

    #[test]
    fn masks_authorization_headers() {
        assert_eq!(
            redact("authorization: Bearer abc.def"),
            "authorization: Bearer [REDACTED]"
        );
        assert_eq!(
            redact("Authorization=token s3cr3t, next"),
            "Authorization=token [REDACTED], next"
        );
    }

    #[test]
    fn leaves_ordinary_lines_borrowed() {
        assert!(matches!(
            redact("synced 12 PRs for acme"),
            Cow::Borrowed("synced 12 PRs for acme")
        ));
    }

    #[test]
    fn the_writer_redacts_before_writing() {
        let mut writer = RedactingWriter::new(Vec::new());
        writeln!(writer, "connect ghp_abcdef1234 ok").unwrap();
        writer.flush().unwrap();
        assert_eq!(
            String::from_utf8(writer.inner).unwrap(),
            "connect ghp_[REDACTED] ok\n"
        );
    }
}
