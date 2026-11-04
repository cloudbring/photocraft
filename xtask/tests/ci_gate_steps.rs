//! #188: `cargo xtask ci` runs the same gate steps, with the same commands, as before the
//! `craft-xtask` swap (#70).
//!
//! OLD is storytold/photocraft @ 327c4cc82e737a3e0af7c147a477126d77525495 (merge-base of cloudbring/photocraft
//! `algo/70-use-craft-xtask` with `origin/main`): its `xtask ci` was built and run by this harness
//! and the transcript recorded in `golden/ci-gate-steps.txt`. The harness puts a `cargo` shim first on
//! PATH (and in $CARGO) that appends each invocation to a log and exits 0, except `cargo metadata` and
//! `cargo tree`, which go to the real cargo (the layering and font-feature checks read them). So no
//! gate really runs: what is compared is the ordered list of cargo commands the steps issue, plus the
//! final "CI summary" line with the step names.
//!
//! To re-record the golden from OLD: check out the SHA above, copy this file to `xtask/tests/` there,
//! and run it with `XTASK_CI_RECORD=1`.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

const GOLDEN: &str = include_str!("golden/ci-gate-steps.txt");

const SHIM: &str = r#"#!/bin/bash
if [ "$1" = metadata ] || [ "$1" = tree ]; then exec "$REAL_CARGO" "$@"; fi
{ printf 'cargo'; for a in "$@"; do printf ' %s' "$a"; done; echo; } >> "$SHIM_LOG"
exit 0
"#;

#[test]
fn ci_issues_the_same_commands_in_the_same_order() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let dir = std::env::temp_dir().join(format!("xtask-ci-shim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let shim = dir.join("cargo");
    std::fs::write(&shim, SHIM).unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    let log = dir.join("cargo.log");
    let _ = std::fs::remove_file(&log);
    let real = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default());
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("ci")
        .current_dir(&root)
        .env("CARGO", &shim)
        .env("REAL_CARGO", &real)
        .env("SHIM_LOG", &log)
        .env("PATH", path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let summary = stdout.lines().find(|l| l.starts_with("CI summary")).unwrap_or("(no CI summary line)");
    let got = format!("{}{summary}\n", std::fs::read_to_string(&log).unwrap_or_default());
    let _ = std::fs::remove_dir_all(&dir);
    if std::env::var_os("XTASK_CI_RECORD").is_some() {
        std::fs::write(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/ci-gate-steps.txt"), &got).unwrap();
        return;
    }
    assert!(out.status.success(), "xtask ci failed:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(got, GOLDEN, "the `xtask ci` step list or commands changed");
}
