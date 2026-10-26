//! One-shot loopback listener for the OAuth redirect (§7.11 step 2–3).
//!
//! It binds exactly the host and port of the configured redirect URI on
//! `127.0.0.1`, answers the first request on the right path whose `state`
//! matches, and closes. Requests on other paths get 404, a foreign `state`
//! gets 400, and both keep the listener waiting.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use url::Url;

use super::SlackAuthError;

/// How long the listener waits for Slack to redirect back (§7.11 step 3).
pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(600);

/// A validated loopback redirect URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectTarget {
    pub uri: String,
    pub port: u16,
    pub path: String,
}

/// Accept only `http://127.0.0.1:<port>/…` or `http://localhost:<port>/…`.
/// The port must be explicit because Slack matches it exactly.
pub fn parse_redirect(uri: &str) -> Result<RedirectTarget, SlackAuthError> {
    let invalid = |why: &str| SlackAuthError::InvalidRedirect(format!("{uri}: {why}"));
    let url = Url::parse(uri.trim()).map_err(|err| invalid(&err.to_string()))?;
    if url.scheme() != "http" {
        return Err(invalid("use http:// with a loopback host"));
    }
    match url.host_str() {
        Some("127.0.0.1" | "localhost") => {}
        _ => return Err(invalid("the host must be 127.0.0.1 or localhost")),
    }
    let port = url.port().ok_or_else(|| invalid("add an explicit port"))?;
    if port == 0 {
        return Err(invalid("port 0 is not allowed"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("remove the query or fragment"));
    }
    Ok(RedirectTarget {
        uri: uri.trim().to_owned(),
        port,
        path: url.path().to_owned(),
    })
}

/// Cancels a pending sign-in from another task.
#[derive(Debug, Clone, Default)]
pub struct CancelHandle(Arc<Notify>);

impl CancelHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.notify_one();
    }

    async fn cancelled(&self) {
        self.0.notified().await;
    }
}

/// The authorization response delivered to the redirect URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    pub code: String,
    pub state: String,
    /// RFC 9207 `iss`, validated by the token exchange when present.
    pub issuer: Option<String>,
}

pub struct Loopback {
    listener: TcpListener,
    target: RedirectTarget,
}

impl Loopback {
    /// Bind before the browser opens so a busy port fails fast.
    pub async fn bind(target: RedirectTarget) -> Result<Self, SlackAuthError> {
        let listener = TcpListener::bind(("127.0.0.1", target.port))
            .await
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied => {
                    SlackAuthError::PortInUse(target.port)
                }
                _ => SlackAuthError::Listener(err.to_string()),
            })?;
        Ok(Self { listener, target })
    }

    /// Wait for the redirect carrying `expected_state`.
    pub async fn wait(
        self,
        expected_state: &str,
        timeout: Duration,
        cancel: &CancelHandle,
    ) -> Result<Callback, SlackAuthError> {
        let serve = async {
            loop {
                let Ok((stream, _)) = self.listener.accept().await else {
                    continue;
                };
                if let Some(result) = self.answer(stream, expected_state).await {
                    return result;
                }
            }
        };
        tokio::select! {
            result = serve => result,
            () = tokio::time::sleep(timeout) => Err(SlackAuthError::Timeout),
            () = cancel.cancelled() => Err(SlackAuthError::Cancelled),
        }
    }

    /// `None` keeps waiting. `Some` ends the flow.
    async fn answer(
        &self,
        mut stream: TcpStream,
        expected_state: &str,
    ) -> Option<Result<Callback, SlackAuthError>> {
        let target = match read_request_target(&mut stream).await {
            Some(target) => target,
            None => {
                respond(&mut stream, 400, "Bad request").await;
                return None;
            }
        };
        let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
            respond(&mut stream, 400, "Bad request").await;
            return None;
        };
        if url.path() != self.target.path {
            respond(&mut stream, 404, "Not found").await;
            return None;
        }
        let param = |name: &str| {
            url.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        if param("state").as_deref() != Some(expected_state) {
            respond(
                &mut stream,
                400,
                "This sign-in link is stale. Start again from PR-Nexus.",
            )
            .await;
            return None;
        }
        if let Some(error) = param("error") {
            respond(
                &mut stream,
                200,
                "Slack sign-in was cancelled. You can close this tab.",
            )
            .await;
            return Some(Err(SlackAuthError::Denied(error)));
        }
        let Some(code) = param("code") else {
            respond(&mut stream, 400, "Missing authorization code").await;
            return None;
        };
        respond(
            &mut stream,
            200,
            "Slack is connected. You can close this tab and return to PR-Nexus.",
        )
        .await;
        Some(Ok(Callback {
            code,
            state: expected_state.to_owned(),
            issuer: param("iss"),
        }))
    }
}

/// Read the request line (`GET <target> HTTP/1.1`) within 8 KiB and 5 s.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let read = async {
        let mut chunk = [0u8; 1024];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 8192 {
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Some(())
    };
    tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .ok()??;
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().next()?;
    let mut parts = line.split(' ');
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    target.starts_with('/').then(|| target.to_owned())
}

async fn respond(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>PR-Nexus</title>\
         <body style=\"font-family:sans-serif;padding:2rem\"><p>{message}</p></body>"
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: text/html; charset=utf-8\r\n\
         content-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_must_be_loopback_with_a_port() {
        let ok = parse_redirect("http://127.0.0.1:47821/mcp/oauth/callback").unwrap();
        assert_eq!(ok.port, 47821);
        assert_eq!(ok.path, "/mcp/oauth/callback");
        assert!(parse_redirect("http://localhost:5555/cb").is_ok());
        for bad in [
            "https://127.0.0.1:47821/cb",
            "http://127.0.0.1/cb",
            "http://example.com:47821/cb",
            "http://0.0.0.0:47821/cb",
            "http://127.0.0.1:47821/cb?x=1",
            "not a url",
        ] {
            assert!(parse_redirect(bad).is_err(), "{bad}");
        }
    }

    async fn get(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).await.unwrap();
        out
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[tokio::test]
    async fn ignores_strays_and_returns_the_matching_callback() {
        let port = free_port();
        let target = parse_redirect(&format!("http://127.0.0.1:{port}/cb")).unwrap();
        let loopback = Loopback::bind(target).await.unwrap();
        let cancel = CancelHandle::new();
        let waiter =
            tokio::spawn(async move { loopback.wait("s1", Duration::from_secs(5), &cancel).await });
        assert!(get(port, "/favicon.ico").await.starts_with("HTTP/1.1 404"));
        assert!(
            get(port, "/cb?code=x&state=other")
                .await
                .starts_with("HTTP/1.1 400")
        );
        assert!(
            get(port, "/cb?code=abc&state=s1")
                .await
                .starts_with("HTTP/1.1 200")
        );
        let callback = waiter.await.unwrap().unwrap();
        assert_eq!(callback.code, "abc");
    }

    #[tokio::test]
    async fn busy_port_denied_timeout_and_cancel() {
        let port = free_port();
        let target = parse_redirect(&format!("http://127.0.0.1:{port}/cb")).unwrap();
        let first = Loopback::bind(target.clone()).await.unwrap();
        let err = Loopback::bind(target.clone()).await.err().unwrap();
        assert_eq!(err, SlackAuthError::PortInUse(port));
        assert!(err.to_string().contains(&format!("Port {port} is in use")));

        let cancel = CancelHandle::new();
        let waiter =
            tokio::spawn(async move { first.wait("s", Duration::from_secs(5), &cancel).await });
        get(port, "/cb?error=access_denied&state=s").await;
        assert_eq!(
            waiter.await.unwrap(),
            Err(SlackAuthError::Denied("access_denied".into()))
        );

        let timed = Loopback::bind(target.clone()).await.unwrap();
        let cancel = CancelHandle::new();
        assert_eq!(
            timed.wait("s", Duration::from_millis(50), &cancel).await,
            Err(SlackAuthError::Timeout)
        );

        let cancelled = Loopback::bind(target).await.unwrap();
        let cancel = CancelHandle::new();
        cancel.cancel();
        assert_eq!(
            cancelled.wait("s", Duration::from_secs(5), &cancel).await,
            Err(SlackAuthError::Cancelled)
        );
    }
}
