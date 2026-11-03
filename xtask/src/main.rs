//! Workspace tooling: `cargo xtask <command>`.
//!
//! Shared tasks use `craft-xtask`. External tools (`cargo`, `curl`, `tar`) are
//! invoked through `std::process::Command`.

mod corpus;
mod corpus_pins;
mod i18n_coverage;
mod perf;
mod pinned;
mod scorecard;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use craft_xtask::{Cargo, ico, layers, stats, version};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  layers          enforce the crate dependency layering (docs/architecture.md §3)
  wasm            cargo check --target wasm32-unknown-unknown for the wasm-safe crates (one call, plus
                  one per optional web feature)
  test            workspace tests (cargo nextest when installed, else cargo test), then doctests
  ci              the CI gate: fmt --check, clippy -D warnings, layers, scorecard --check, tests
                  (nextest --profile ci), panic_hunt, doctests and wasm, with --locked --features heif
                  (stops at first failure), then the i18n-coverage report
  corpus [--all | --pngsuite | --psd | --psd-tools | --pixls | --atn | --photoshop | --gimp] [--local] [--update-manifest]
                  show where test corpora live and their pins (xtask/src/corpus_pins.rs), or fetch
                  them into corpus/ (pinned commits, sha256-verified; --all = every corpus but pixls,
                  --photoshop/--gimp --local copy from ../photocraft-corpus or $PHOTOCRAFT_CORPUS_REPO)
  test-corpus [-p <crate>]... [--changed] [--pixls] [--local] [-- <test args>]
                  fetch every corpus, then cargo test --release --features corpus on the corpus
                  crates; --changed runs only if psd/io/codecs/compose/gpu/text/format/raw changed;
                  --pixls (or -p raw) also fetches corpus/pixls and runs the raw corpus tests;
                  --local takes corpus/photoshop and corpus/gimp from the photocraft-corpus authoring clone
  stats [--exact] count tests and lines per crate (--exact: ask the test harness via `-- --list`)
  parity          Photoshop menu parity; rewrites docs/parity-checklist.md
  i18n-coverage   report UI translation coverage and the missing strings per registered language
                  (the last `ci` step; it never fails)
  perf [--quick] [--update-baseline] [--threshold PCT] [--bench NAME]... [--skip-build] [--reuse]
                  run the release benches, merge them by scenario id into target/perf/results.json,
                  check perf/budgets.toml and perf/baseline.json (non-zero on a broken budget or regression)
  scorecard [--check]
                  regenerate docs/scorecard.md (--check: fail if it is stale)
  version [set X.Y.Z[-pre]]
                  print the workspace version, or set it (Cargo.toml + Cargo.lock)
  ico <out.ico> <in.png>...
                  pack square PNGs (<= 256 px) into a Windows .ico (see packaging/icons.sh)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let task_args = args.get(1..).unwrap_or_default();
    let stats_args = if rest.contains(&"--exact") { vec!["--exact".into()] } else { Vec::new() };
    let result = match args.first().map(String::as_str) {
        Some("layers") => cmd_layers(),
        Some("wasm") => cmd_wasm(),
        Some("test") => cmd_test().and_then(|()| cmd_doctest()),
        Some("ci") => cmd_ci(),
        Some("corpus") => corpus::cmd(&rest),
        Some("test-corpus") => corpus::test_cmd(&rest),
        Some("stats") => stats::run(&stats_config(&root()), &stats_args),
        Some("parity") => cmd_parity(),
        Some("i18n-coverage") => i18n_coverage::run(&root()),
        Some("perf") => perf::run(&root(), &rest),
        Some("scorecard") => scorecard::run(&root(), &rest),
        Some("version") => version::run(&version_config(&root()), task_args),
        Some("ico") => ico::run(&ico::Config::default(), task_args),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (parent of the xtask crate).
pub fn root() -> PathBuf {
    let dir = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).filter(|dir| dir.join("Cargo.toml").is_file());
    let dir = dir.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    dir.parent().unwrap_or(Path::new(".")).to_path_buf()
}

pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

pub fn version_config(root: &Path) -> version::Config {
    version::Config { cargo: Cargo { root: root.into(), ..Cargo::default() }, strict_prerelease: true, ..version::Config::default() }
}

pub fn stats_config(root: &Path) -> stats::Config {
    stats::Config { cargo: Cargo { root: root.into(), ..Cargo::default() }, follow_symlinks: true, ..stats::Config::default() }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).into()).collect()
}

pub fn layers_config(root: &Path) -> layers::Config {
    layers::Config {
        root: root.into(),
        layers: [
            (
                "L0",
                names(&[
                    "photocraft-geom",
                    "photocraft-cms",
                    "photocraft-color",
                    "photocraft-raster",
                    "photocraft-psd",
                    "photocraft-codecs",
                    "photocraft-heif",
                    "photocraft-ipp",
                    "photocraft-raw",
                    "photocraft-affinity",
                    "photocraft-adobe-assets",
                    "photocraft-tablet",
                ]),
            ),
            ("L1", names(&["photocraft-doc"])),
            ("L2", names(&["photocraft-ops", "photocraft-paint", "photocraft-algo", "photocraft-text", "photocraft-vector"])),
            ("L3", names(&["photocraft-compose", "photocraft-gpu", "photocraft-format"])),
            ("L4", names(&["photocraft-io", "photocraft-tools", "photocraft-viewport", "photocraft-ml", "photocraft-plugins"])),
            ("L5", names(&["photocraft-engine"])),
            ("L6", names(&["photocraft-ui-egui", "photocraft-automation", "photocraft-platform", "photocraft-testkit"])),
        ]
        .into_iter()
        .map(|(label, members)| (label.into(), members))
        .collect(),
        exempt: names(&["photocraft", "photocraft-cli", "photocraft-web", "photocraft-web-boot", "xtask"]),
        standalone: names(&[
            "photocraft-psd",
            "photocraft-codecs",
            "photocraft-heif",
            "photocraft-ipp",
            "photocraft-raw",
            "photocraft-affinity",
            "photocraft-adobe-assets",
            "photocraft-tablet",
        ]),
        testkits: names(&["photocraft-testkit"]),
        allowed_edges: [
            ("photocraft-cms", "photocraft-geom"),
            ("photocraft-color", "photocraft-geom"),
            ("photocraft-color", "photocraft-cms"),
            ("photocraft-raster", "photocraft-geom"),
            ("photocraft-raster", "photocraft-cms"),
            ("photocraft-raster", "photocraft-color"),
            ("photocraft-gpu", "photocraft-compose"),
            ("photocraft-codecs", "photocraft-heif"),
        ]
        .into_iter()
        .map(|(from, to)| (from.into(), to.into()))
        .collect(),
        external_rules: vec![layers::ExternalRule {
            dependencies: names(&["egui", "eframe", "winit", "egui_kittest", "craft-ui", "rfd", "bevy*"]),
            allowed_from: names(&["photocraft-ui-egui", "photocraft-automation", "photocraft-platform", "photocraft-testkit"]),
            allowed_dev_from: Vec::new(),
        }],
        ..layers::Config::default()
    }
}

fn describe_layer<'a>(cfg: &'a layers::Config, name: &str) -> &'a str {
    if cfg.exempt.iter().any(|m| m == name) {
        "exempt"
    } else if cfg.standalone.iter().any(|m| m == name) {
        "L0 standalone"
    } else if cfg.testkits.iter().any(|m| m == name) {
        "testkit"
    } else {
        cfg.layers.iter().find(|(_, members)| members.iter().any(|m| m == name)).map(|(label, _)| label.as_str()).unwrap_or("UNREGISTERED")
    }
}

fn cmd_layers() -> Result<(), String> {
    let cfg = layers_config(&root());
    let report = layers::compute(&cfg, &layers::Args::default())?;
    println!("Dependency layering (docs/architecture.md §3)\n");
    println!("{:<28} {:<14} workspace deps", "crate", "layer");
    for c in &report.crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", d.name.strip_prefix("photocraft-").unwrap_or(&d.name))
            })
            .collect();
        println!("{:<28} {:<14} {}", c.name, describe_layer(&cfg, &c.name), ws.join(", "));
    }
    println!();
    if report.ok {
        println!("OK: {} crates, no layering violations.", report.crates.len());
        Ok(())
    } else {
        println!("{} violation(s):", report.violations.len());
        for v in &report.violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", report.violations.len()))
    }
}

/// Workspace packages that must build for wasm32: all L0–L5 crates plus
/// the egui shell.
fn wasm_set() -> Result<Vec<String>, String> {
    let cfg = layers_config(&root());
    let crates = layers::from_metadata(&metadata()?)?;
    Ok(crates.into_iter().filter(|c| cfg.layers.iter().take(6).any(|(_, m)| m.contains(&c.name)) || c.name == "photocraft-ui-egui").map(|c| c.name).collect())
}

/// Optional features that official builds enable and the web app ships, also checked for wasm32:
/// (package, feature).
const WASM_FEATURES: &[(&str, &str)] = &[("photocraft-codecs", "heif")];

/// One `cargo check` for the whole set: each crate is checked once, and `--keep-going` still
/// reports every failing crate. Features unify across the set, so each crate is checked with the
/// union of the features the set enables. The optional features get a call each, so the crates
/// are also checked without them.
fn cmd_wasm() -> Result<(), String> {
    let set = wasm_set()?;
    if set.is_empty() {
        return Err("no wasm-safe crates found in the workspace metadata".into());
    }
    let mut results = Vec::new();
    let mut c = cargo();
    c.args(["check", "--target", "wasm32-unknown-unknown", "--keep-going"]);
    for pkg in &set {
        c.args(["-p", pkg]);
    }
    let ok = run(c, &format!("cargo check --target wasm32-unknown-unknown --keep-going -p {}", set.join(" -p "))).is_ok();
    results.push((format!("{} crates", set.len()), ok));
    for (pkg, feature) in WASM_FEATURES {
        let mut c = cargo();
        c.args(["check", "--target", "wasm32-unknown-unknown", "-p", pkg, "--features", feature]);
        let ok = run(c, &format!("cargo check --target wasm32-unknown-unknown -p {pkg} --features {feature}")).is_ok();
        results.push((format!("{pkg} --features {feature}"), ok));
    }
    println!("\nwasm32-unknown-unknown check:");
    for (p, ok) in &results {
        println!("  {:<6} {p}", if *ok { "ok" } else { "FAIL" });
    }
    let failed = results.iter().filter(|r| !r.1).count();
    if failed == 0 { Ok(()) } else { Err(format!("{failed} wasm check(s) failed")) }
}

/// Test targets for the test build. Examples are left out so they aren't linked; clippy
/// `--all-targets` still compiles them.
const TEST_TARGETS: [&str; 4] = ["--workspace", "--lib", "--bins", "--tests"];

fn nextest_available() -> bool {
    cargo().args(["nextest", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn cmd_test() -> Result<(), String> {
    let mut c = cargo();
    let tool = if nextest_available() {
        c.args(["nextest", "run"]);
        "nextest run"
    } else {
        eprintln!("note: cargo-nextest is not installed, running cargo test (cargo install cargo-nextest --locked)");
        c.arg("test");
        "test"
    };
    c.args(TEST_TARGETS);
    run(c, &format!("cargo {tool} {}", TEST_TARGETS.join(" ")))
}

/// What the CI gate adds to every build: `--locked`, and the optional photocraft-heif crate that
/// official builds open HEIC photos with.
const GATE_ARGS: [&str; 4] = ["--workspace", "--locked", "--features", "heif"];

/// The gate's tests. nextest's `ci` profile stops at the first failure and terminates hung tests
/// (.config/nextest.toml). `panic_hunt` runs only the engine's `#[ignore]`d panic hunt (AGENTS.md
/// Rule 9) with the same packages, features and targets as the tests, so nothing rebuilds.
fn cmd_gate_tests(panic_hunt: bool) -> Result<(), String> {
    let mut c = cargo();
    if nextest_available() {
        c.args(["nextest", "run", "--profile", "ci"]).args(GATE_ARGS).args(&TEST_TARGETS[1..]);
        if panic_hunt {
            c.args(["--run-ignored", "only", "-E", "binary_id(photocraft-engine::panic_hunt)"]);
        }
    } else if panic_hunt {
        // `-- --ignored` across the workspace would run every ignored test, not only the hunt. The
        // engine has no `heif` feature of its own; it reaches the decoder through photocraft-io.
        c.args(["test", "-p", "photocraft-engine", "--locked", "--features", "photocraft-io/heif", "--test", "panic_hunt", "--", "--ignored"]);
    } else {
        eprintln!("note: cargo-nextest is not installed, running cargo test (cargo install cargo-nextest --locked)");
        c.arg("test").args(GATE_ARGS).args(&TEST_TARGETS[1..]);
    }
    let what = format!("cargo {}", c.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    run(c, &what)
}

/// nextest does not run doctests.
fn cmd_doctest() -> Result<(), String> {
    let mut c = cargo();
    c.args(["test", "--workspace", "--doc"]);
    run(c, "cargo test --workspace --doc")
}

fn cmd_parity() -> Result<(), String> {
    let mut c = cargo();
    c.args(["run", "-q", "-p", "photocraft-ui-egui", "--example", "parity", "--", "--write", "docs/parity-checklist.md"]);
    run(c, "cargo run -p photocraft-ui-egui --example parity")
}

fn cmd_ci() -> Result<(), String> {
    type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);
    let steps: Vec<Step> = vec![
        (
            "fmt",
            Box::new(|| {
                let mut c = cargo();
                c.args(["fmt", "--all", "--", "--check"]);
                run(c, "cargo fmt --all -- --check")
            }),
        ),
        (
            "clippy",
            Box::new(|| {
                let mut c = cargo();
                c.arg("clippy").args(GATE_ARGS).args(["--all-targets", "--keep-going", "--", "-D", "warnings"]);
                run(c, "cargo clippy --workspace --locked --features heif --all-targets --keep-going -- -D warnings")
            }),
        ),
        ("layers", Box::new(cmd_layers)),
        ("scorecard", Box::new(|| scorecard::run(&root(), &["--check"]))),
        ("test", Box::new(|| cmd_gate_tests(false))),
        ("panic_hunt", Box::new(|| cmd_gate_tests(true))),
        (
            "doctest",
            Box::new(|| {
                let mut c = cargo();
                c.arg("test").args(GATE_ARGS).arg("--doc");
                run(c, "cargo test --workspace --locked --features heif --doc")
            }),
        ),
        ("wasm", Box::new(cmd_wasm)),
        (
            "i18n",
            Box::new(|| {
                // Lists missing translations for the log; a missing string shows in English, so it never fails CI.
                if let Err(e) = i18n_coverage::run(&root()) {
                    eprintln!("warning: i18n coverage report failed: {e}");
                }
                Ok(())
            }),
        ),
    ];
    let mut done = Vec::new();
    for (name, f) in &steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = f() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_preserve_photocraft_policies() {
        let vcfg = version_config(&root());
        assert!(vcfg.strict_prerelease);
        let sample = "[workspace.package]\nversion = \"0.1.0\"\n";
        assert!(version::rewrite(&vcfg, sample, "1.0.0-01").is_err());
        assert!(version::rewrite(&vcfg, sample, "1.0.0-rc.1").is_ok());

        let scfg = stats_config(&root());
        assert!(scfg.follow_symlinks);

        let lcfg = layers_config(&root());
        let edge = |from: &str, to: &str, kind: layers::DepKind, workspace: bool| layers::Crate {
            name: from.into(),
            deps: vec![layers::Dep { name: to.into(), kind, workspace }],
        };
        assert!(layers::check(&lcfg, &[edge("photocraft-codecs", "photocraft-heif", layers::DepKind::Normal, true)]).unwrap().is_empty());
        assert!(!layers::check(&lcfg, &[edge("photocraft-heif", "photocraft-codecs", layers::DepKind::Normal, true)]).unwrap().is_empty());
        assert!(!layers::check(&lcfg, &[edge("photocraft-engine", "egui", layers::DepKind::Normal, false)]).unwrap().is_empty());
    }
}
