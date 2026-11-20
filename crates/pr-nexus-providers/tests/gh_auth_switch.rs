//! A fake `gh` on PATH. Its own test binary, so changing PATH cannot race
//! other tests.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use pr_nexus_providers::github::{read_gh_token, read_gh_token_now};

#[test]
fn connect_after_gh_auth_switch_reads_the_new_active_account() {
    let dir = std::env::temp_dir().join(format!("prn-gh-switch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let active = dir.join("active");
    std::fs::write(&active, "fake-token-alice").unwrap();
    let gh = dir.join("gh");
    std::fs::write(&gh, format!("#!/bin/sh\n/bin/cat {}\n", active.display())).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: the only test in this binary, so nothing else reads PATH.
    unsafe { std::env::set_var("PATH", &dir) };

    // The wizard opens: bootstrap's gh probe fills the cache.
    assert_eq!(read_gh_token().token(), Some("fake-token-alice"));
    // The user runs `gh auth switch`.
    std::fs::write(&active, "fake-token-bob").unwrap();
    // Background reads keep the cached token for its TTL.
    assert_eq!(read_gh_token().token(), Some("fake-token-alice"));
    // Connect GitHub and the wizard's re-check read gh again.
    assert_eq!(read_gh_token_now(None).token(), Some("fake-token-bob"));
    assert_eq!(read_gh_token().token(), Some("fake-token-bob"));
}
