//! Records which build of crabsim this is, the git commit it was built from, so every run in the
//! results store says which build ran it. `-dirty` marks uncommitted changes, and `unknown` a
//! build outside git, such as from a downloaded source archive.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn main() {
    let build = match git(&["rev-parse", "--short=7", "HEAD"]) {
        Some(commit) => {
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|status| !status.is_empty());
            if dirty { format!("{commit}-dirty") } else { commit }
        }
        None => "unknown".to_string(),
    };
    println!("cargo:rustc-env=CRABSIM_BUILD={build}");
    // Built again when the commit or the staged files change.
    for path in ["../.git/HEAD", "../.git/refs", "../.git/index"] {
        println!("cargo:rerun-if-changed={path}");
    }
}
