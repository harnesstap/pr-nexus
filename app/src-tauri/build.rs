use std::process::Command;

fn main() {
    embed_git_sha();
    tauri_build::build()
}

/// `PR_NEXUS_GIT_SHA`: the commit this binary was built from, so a dev build
/// can tell how far behind `main` it is. Empty outside a git checkout.
fn embed_git_sha() {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|text| text.trim().to_owned())
    };
    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    println!("cargo:rustc-env=PR_NEXUS_GIT_SHA={sha}");
    // Rebuild when HEAD moves (commit, checkout, pull).
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/logs/HEAD");
    }
}
