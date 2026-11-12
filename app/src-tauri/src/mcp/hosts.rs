//! DNS-rebinding allowlist for the local MCP server.
//!
//! `127.0.0.1`, `localhost`, and `[::1]` on the bound port are always
//! allowed. Saved entries add hostnames (or an IPv4 address), with or
//! without a port. A wildcard, a scheme, or a path is not an entry, and
//! nothing here turns the check off.

use std::net::Ipv6Addr;

/// One saved host, already trimmed and lowercased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPattern {
    host: String,
    port: Option<u16>,
}

impl HostPattern {
    fn matches(&self, host: &str, port: Option<u16>) -> bool {
        self.host == host && self.port.is_none_or(|expected| port == Some(expected))
    }

    /// `host` or `host:port`, the form both the settings list and rmcp store.
    fn entry(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        }
    }
}

/// Drops invalid entries, lowercases the rest, and removes duplicates.
pub fn normalize(entries: &[String]) -> Vec<String> {
    let mut hosts: Vec<String> = entries
        .iter()
        .map(String::as_str)
        .filter_map(parse_entry)
        .map(|pattern| pattern.entry())
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

/// Patterns for entries that parse. Invalid strings are skipped.
pub fn compile(entries: &[String]) -> Vec<HostPattern> {
    entries
        .iter()
        .map(String::as_str)
        .filter_map(parse_entry)
        .collect()
}

/// Hosts rmcp should accept for this process: loopback on `bound_port`,
/// plus each saved entry. A name without a port matches any port there too.
pub fn rmcp_allowlist(bound_port: u16, extra: &[HostPattern]) -> Vec<String> {
    let mut hosts = vec![
        format!("127.0.0.1:{bound_port}"),
        format!("localhost:{bound_port}"),
        format!("[::1]:{bound_port}"),
    ];
    hosts.extend(extra.iter().map(HostPattern::entry));
    hosts
}

/// Whether `header` (a Host value) may reach the server bound to `bound_port`.
pub fn host_allowed(header: &str, bound_port: u16, extra: &[HostPattern]) -> bool {
    let Some((host, port)) = parse_request_host(header) else {
        return false;
    };
    (is_loopback(&host) && port == Some(bound_port))
        || extra.iter().any(|pattern| pattern.matches(&host, port))
}

/// Whether an Origin value is the `http` origin of an allowed host.
/// A missing Origin is the caller's concern; this is only for a present one.
pub fn origin_allowed(origin: &str, bound_port: u16, extra: &[HostPattern]) -> bool {
    let origin = origin.trim();
    let Some(rest) = strip_http(origin) else {
        return false;
    };
    if rest.contains(['/', '?', '#', '@']) {
        return false;
    }
    host_allowed(rest, bound_port, extra)
}

fn strip_http(origin: &str) -> Option<&str> {
    let prefix = "http://";
    if origin.len() >= prefix.len() && origin[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&origin[prefix.len()..])
    } else {
        None
    }
}

fn parse_entry(raw: &str) -> Option<HostPattern> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 300 || forbidden(raw) {
        return None;
    }
    let (name, port) = match raw.rsplit_once(':') {
        Some((name, port)) => {
            // A colon inside the name is an IPv6 literal or a scheme. Extra
            // hosts are names and IPv4 addresses; loopback IPv6 is built in.
            if name.is_empty() || name.contains(':') {
                return None;
            }
            (name, Some(parse_port(port)?))
        }
        None => (raw, None),
    };
    Some(HostPattern {
        host: canonical_name(name)?,
        port,
    })
}

fn parse_request_host(raw: &str) -> Option<(String, Option<u16>)> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 300 || forbidden(raw) {
        return None;
    }
    if let Some(rest) = raw.strip_prefix('[') {
        let (inner, after) = rest.split_once(']')?;
        let host = inner.to_ascii_lowercase();
        if host.parse::<Ipv6Addr>().is_err() {
            return None;
        }
        let port = if after.is_empty() {
            None
        } else {
            Some(parse_port(after.strip_prefix(':')?)?)
        };
        return Some((host, port));
    }
    if raw.matches(':').count() > 1 {
        return None;
    }
    let (name, port) = match raw.rsplit_once(':') {
        Some((name, port)) => (name, Some(parse_port(port)?)),
        None => (raw, None),
    };
    Some((canonical_name(name)?, port))
}

fn forbidden(raw: &str) -> bool {
    raw.contains(['*', '/', '\\', '?', '#', '@', ' ', '\t', '\n', '\r']) || raw.contains("://")
}

fn parse_port(raw: &str) -> Option<u16> {
    if raw.is_empty()
        || raw.len() > 5
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let port = raw.parse::<u16>().ok()?;
    (port >= 1).then_some(port)
}

fn canonical_name(name: &str) -> Option<String> {
    let labels: Vec<&str> = name.split('.').collect();
    if labels.len() == 4
        && labels
            .iter()
            .all(|label| label.bytes().all(|b| b.is_ascii_digit()))
    {
        return canonical_ipv4(name);
    }
    let lower = name.to_ascii_lowercase();
    is_dns_name(&lower).then_some(lower)
}

fn canonical_ipv4(name: &str) -> Option<String> {
    let mut octets = [0u8; 4];
    for (index, part) in name.split('.').enumerate() {
        if part.is_empty()
            || part.len() > 3
            || (part.len() > 1 && part.starts_with('0'))
            || !part.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        octets[index] = part.parse().ok()?;
    }
    Some(std::net::Ipv4Addr::from(octets).to_string())
}

fn is_dns_name(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 || !host.is_ascii() {
        return false;
    }
    host.split('.').all(|label| {
        let bytes = label.as_bytes();
        let len = bytes.len();
        (1..=63).contains(&len)
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
            && bytes[0] != b'-'
            && bytes[len - 1] != b'-'
    })
}

fn is_loopback(host: &str) -> bool {
    host == "127.0.0.1"
        || host == "localhost"
        || host
            .parse::<Ipv6Addr>()
            .ok()
            .is_some_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT: u16 = 49708;

    fn patterns(entries: &[&str]) -> Vec<HostPattern> {
        let owned: Vec<String> = entries.iter().map(|entry| (*entry).to_owned()).collect();
        compile(&normalize(&owned))
    }

    #[test]
    fn loopback_matches_this_port_only_and_ignores_case() {
        assert!(host_allowed("127.0.0.1:49708", PORT, &[]));
        assert!(host_allowed("LocalHost:49708", PORT, &[]));
        assert!(!host_allowed("127.0.0.1:80", PORT, &[]));
        assert!(!host_allowed("localhost:80", PORT, &[]));
        assert!(!host_allowed("127.0.0.1", PORT, &[]));
        assert!(!host_allowed("127.1.1.1:49708", PORT, &[]));
        assert!(origin_allowed("HTTP://localhost:49708", PORT, &[]));
        assert!(origin_allowed("http://127.0.0.1:49708", PORT, &[]));
        assert!(!origin_allowed("https://localhost:49708", PORT, &[]));
        assert!(!origin_allowed("http://127.0.0.1:80", PORT, &[]));
    }

    #[test]
    fn ipv6_loopback_matches_this_port_only() {
        assert!(host_allowed("[::1]:49708", PORT, &[]));
        assert!(host_allowed("[0:0:0:0:0:0:0:1]:49708", PORT, &[]));
        assert!(!host_allowed("[::1]", PORT, &[]));
        assert!(!host_allowed("[::1]:80", PORT, &[]));
        assert!(!host_allowed("[2001:db8::1]:49708", PORT, &[]));
        assert!(!host_allowed("[::ffff:127.0.0.1]:49708", PORT, &[]));
        assert!(origin_allowed("http://[::1]:49708", PORT, &[]));
        assert!(!origin_allowed("https://[::1]:49708", PORT, &[]));
        assert!(!origin_allowed("http://[2001:db8::1]:49708", PORT, &[]));
    }

    #[test]
    fn a_name_without_a_port_matches_any_port_ignoring_case() {
        let extra = patterns(&[" Host.Docker.Internal "]);
        assert!(host_allowed("HOST.DOCKER.INTERNAL:49708", PORT, &extra));
        assert!(host_allowed("host.docker.internal:1", PORT, &extra));
        assert!(host_allowed("host.docker.internal", PORT, &extra));
        assert!(origin_allowed(
            "http://host.docker.internal:49708",
            PORT,
            &extra
        ));
        assert!(origin_allowed("http://Host.Docker.Internal", PORT, &extra));
        assert_eq!(
            rmcp_allowlist(PORT, &extra),
            vec![
                "127.0.0.1:49708".to_owned(),
                "localhost:49708".to_owned(),
                "[::1]:49708".to_owned(),
                "host.docker.internal".to_owned(),
            ]
        );
    }

    #[test]
    fn a_name_with_a_port_matches_only_that_port() {
        let extra = patterns(&["host.docker.internal:49708"]);
        assert!(host_allowed("host.docker.internal:49708", PORT, &extra));
        assert!(origin_allowed(
            "http://host.docker.internal:49708",
            PORT,
            &extra
        ));
        assert!(!host_allowed("host.docker.internal:80", PORT, &extra));
        assert!(!host_allowed("host.docker.internal", PORT, &extra));
        assert!(!origin_allowed(
            "http://host.docker.internal:80",
            PORT,
            &extra
        ));
        assert!(!origin_allowed("http://host.docker.internal", PORT, &extra));
    }

    #[test]
    fn other_hosts_are_rejected() {
        let extra = patterns(&["host.docker.internal"]);
        assert!(!host_allowed("rebind.acme-labs.example", PORT, &extra));
        assert!(!host_allowed(
            "rebind.acme-labs.example:49708",
            PORT,
            &extra
        ));
        assert!(!host_allowed("evil.acme-labs.example", PORT, &[]));
        assert!(!host_allowed("", PORT, &extra));
        assert!(!origin_allowed(
            "http://evil.acme-labs.example",
            PORT,
            &extra
        ));
        assert!(!origin_allowed(
            "http://host.docker.internal:49708/mcp",
            PORT,
            &extra
        ));
        assert!(!origin_allowed(
            "http://host.docker.internal:49708?x=1",
            PORT,
            &extra
        ));
    }

    #[test]
    fn a_star_scheme_or_path_is_dropped_and_does_not_open_the_check() {
        let raw = vec![
            "*".to_owned(),
            " * ".to_owned(),
            "*.docker.internal".to_owned(),
            "http://host.docker.internal".to_owned(),
            "host.docker.internal/mcp".to_owned(),
            "host.docker.internal:".to_owned(),
            "host.docker.internal:0".to_owned(),
            "host.docker.internal:65536".to_owned(),
            "host.docker.internal:049708".to_owned(),
            "172.17.0.01".to_owned(),
            "999.0.0.1".to_owned(),
            "  host.docker.internal  ".to_owned(),
            "HOST.DOCKER.INTERNAL".to_owned(),
            "host.docker.internal:49708".to_owned(),
            "172.17.0.1".to_owned(),
        ];
        let saved = normalize(&raw);
        assert_eq!(
            saved,
            vec![
                "172.17.0.1".to_owned(),
                "host.docker.internal".to_owned(),
                "host.docker.internal:49708".to_owned(),
            ]
        );
        let open = compile(&normalize(&["*".to_owned()]));
        assert!(open.is_empty());
        assert!(!host_allowed("evil.acme-labs.example:49708", PORT, &open));
        assert!(!host_allowed("*", PORT, &open));
        assert_eq!(normalize(&saved), saved);
    }
}
