//! Build metadata exported to the crate as `KB_*` environment variables.

#[path = "src/utc.rs"]
mod utc;
#[path = "src/build_support.rs"]
mod build_support;

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Trimmed stdout of a git command, or None when git is missing or fails.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    let sha = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let count = git(&["rev-list", "--count", "HEAD"]).and_then(|c| c.parse::<u32>().ok()).unwrap_or(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let git_time = git(&["log", "-1", "--format=%ct"]).and_then(|t| t.parse::<i64>().ok());
    let epoch = build_support::build_time_from_epoch(std::env::var("SOURCE_DATE_EPOCH").ok().as_deref(), git_time, now);
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "release".into());
    let build_type = build_support::build_type_from_out_dir(&std::env::var("OUT_DIR").unwrap_or_default(), &profile);
    println!("cargo:rustc-env=KB_GIT_SHA={sha}");
    println!("cargo:rustc-env=KB_COMMIT_COUNT={count}");
    println!("cargo:rustc-env=KB_BUILD_TIME={}", utc::rfc3339(epoch));
    println!("cargo:rustc-env=KB_BUILD_TYPE={build_type}");
    println!("cargo:rerun-if-env-changed=KB_CHANNEL");
    let nightly = std::env::var("KB_CHANNEL").is_ok_and(|channel| channel == "nightly");
    println!("cargo:rustc-env=KB_APP_ID=io.github.kurobayomi.Kurobayomi{}", if nightly { ".Nightly" } else { "" });
}
