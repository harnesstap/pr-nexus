//! OAuth HTTP adapter for Slack's token endpoint.
//!
//! Slack answers OAuth errors with HTTP 200 and `{"ok":false,"error":"…"}`.
//! The OAuth library expects RFC 6749 errors (HTTP 400 with `error`), so this
//! wrapper rewrites those bodies. Every other response passes through.

use rmcp::transport::auth::{
    AuthError, OAuthHttpClient, OAuthHttpClientFuture, OAuthHttpRequest, default_oauth_http_client,
};

pub struct SlackOAuthHttpClient {
    inner: Box<dyn OAuthHttpClient>,
}

impl SlackOAuthHttpClient {
    pub fn new() -> Result<Self, AuthError> {
        Ok(Self {
            inner: Box::new(default_oauth_http_client()?),
        })
    }
}

impl OAuthHttpClient for SlackOAuthHttpClient {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            let response = self.inner.execute(request).await?;
            Ok(rewrite_slack_error(response))
        })
    }
}

/// Turn a Slack `ok:false` body into an RFC 6749 error response.
pub fn rewrite_slack_error(response: http::Response<Vec<u8>>) -> http::Response<Vec<u8>> {
    if !response.status().is_success() {
        return response;
    }
    let Ok(body) = serde_json::from_slice::<serde_json::Value>(response.body()) else {
        return response;
    };
    if body.get("ok") != Some(&serde_json::Value::Bool(false)) {
        return response;
    }
    let slack_error = body
        .get("error")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown_error");
    let error = match slack_error {
        "invalid_refresh_token"
        | "token_expired"
        | "token_revoked"
        | "invalid_code"
        | "code_already_used" => "invalid_grant",
        "invalid_client_id" | "bad_client_secret" | "invalid_client" => "invalid_client",
        "bad_redirect_uri" | "invalid_grant_type" => "invalid_request",
        _ => "invalid_request",
    };
    let rewritten = serde_json::json!({
        "error": error,
        "error_description": format!("Slack: {slack_error}"),
    });
    let mut out = http::Response::new(rewritten.to_string().into_bytes());
    *out.status_mut() = http::StatusCode::BAD_REQUEST;
    out.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, body: &str) -> http::Response<Vec<u8>> {
        let mut response = http::Response::new(body.as_bytes().to_vec());
        *response.status_mut() = http::StatusCode::from_u16(status).unwrap();
        response
    }

    #[test]
    fn slack_ok_false_becomes_an_oauth_error() {
        let out = rewrite_slack_error(response(
            200,
            r#"{"ok":false,"error":"invalid_refresh_token"}"#,
        ));
        assert_eq!(out.status(), 400);
        let body: serde_json::Value = serde_json::from_slice(out.body()).unwrap();
        assert_eq!(body["error"], "invalid_grant");
        assert_eq!(body["error_description"], "Slack: invalid_refresh_token");
        let out = rewrite_slack_error(response(200, r#"{"ok":false,"error":"bad_client_secret"}"#));
        let body: serde_json::Value = serde_json::from_slice(out.body()).unwrap();
        assert_eq!(body["error"], "invalid_client");
    }

    #[test]
    fn success_and_non_json_pass_through() {
        let ok = r#"{"ok":true,"access_token":"xoxp-test","token_type":"user"}"#;
        assert_eq!(rewrite_slack_error(response(200, ok)).body(), ok.as_bytes());
        assert_eq!(rewrite_slack_error(response(200, "<html>")).status(), 200);
        assert_eq!(
            rewrite_slack_error(response(500, r#"{"ok":false}"#)).status(),
            500
        );
    }
}
