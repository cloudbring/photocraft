use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use craft_xtask::{Cargo, ico, layers, stats, version};
use serde_json::json;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("photocraft-70-golden-{}-{}", std::process::id(), NEXT_TEMP.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, rel: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn xtask_cmd(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask")).args(args).current_dir(cwd).env("CARGO_MANIFEST_DIR", Path::new(env!("CARGO_MANIFEST_DIR"))).output().unwrap()
}

fn assert_output(output: Output, expected: &[u8]) {
    assert!(output.status.success(), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, expected);
}

fn version_config(root: &Path) -> version::Config {
    version::Config { cargo: Cargo { root: root.into(), ..Cargo::default() }, strict_prerelease: true, ..version::Config::default() }
}

fn stats_config(root: &Path) -> stats::Config {
    stats::Config { cargo: Cargo { root: root.into(), ..Cargo::default() }, follow_symlinks: true, ..stats::Config::default() }
}

#[test]
fn layers_matches_golden_byte_for_byte() {
    assert_output(xtask_cmd(&["layers"], &repo_root()), include_bytes!("golden/layers.stdout"));
}

#[test]
fn stats_replays_golden_counts_byte_for_byte_and_matches_live_tree() {
    let temp = Temp::new();
    let golden = include_str!("golden/stats.stdout");
    let mut packages = Vec::new();
    for line in golden.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 5 || parts[0] == "crate" || parts[0] == "TOTAL" {
            continue;
        }
        let name = parts[0];
        let files: usize = parts[1].parse().unwrap();
        let lines: usize = parts[2].parse().unwrap();
        let tests: usize = parts[3].parse().unwrap();
        let props: usize = parts[4].parse().unwrap();
        let mut src = "#[test]\n".repeat(tests);
        src += &"proptest! {}\n".repeat(props);
        src += &"\n".repeat(lines - tests - props);
        temp.write(format!("{name}/src/0.rs"), src);
        for idx in 1..files {
            temp.write(format!("{name}/src/{idx}.rs"), "");
        }
        packages.push(json!({"name": name, "manifest_path": format!("{name}/Cargo.toml")}));
    }
    let report = stats::report(&stats_config(&temp.0), &json!({"packages": packages}), false).unwrap();
    assert_eq!(report.as_bytes(), include_bytes!("golden/stats.stdout"));

    let live = xtask_cmd(&["stats"], &repo_root());
    assert!(live.status.success(), "{}", String::from_utf8_lossy(&live.stderr));
    let filter = |text: &str| text.lines().filter(|line| !line.starts_with("xtask ") && !line.starts_with("TOTAL ")).collect::<Vec<_>>().join("\n");
    assert_eq!(filter(&String::from_utf8(live.stdout).unwrap()), filter(golden));
}

#[test]
fn version_show_and_dry_run_plan_match_golden_and_strict_prerelease() {
    assert_output(xtask_cmd(&["version"], &repo_root()), include_bytes!("golden/version.stdout"));

    let temp = Temp::new();
    let root_manifest = std::fs::read(repo_root().join("Cargo.toml")).unwrap();
    temp.write("Cargo.toml", &root_manifest);

    let cfg = version_config(&temp.0);
    let show_plan = version::compute(&cfg, &version::Args::Show).unwrap();
    assert_eq!(version::render(&show_plan).as_bytes(), include_bytes!("golden/version.stdout"));

    let set_plan = version::compute(&cfg, &version::Args::Set("1.2.3-rc.1".into())).unwrap();
    assert_eq!(version::render(&set_plan).as_bytes(), include_bytes!("golden/version-set.stdout"));
    assert_eq!(set_plan.edits.len(), 1);
    assert_eq!(set_plan.edits[0].2.as_bytes(), include_bytes!("golden/version-after.toml"));
    assert_eq!(std::fs::read(temp.0.join("Cargo.toml")).unwrap(), root_manifest);

    let sample = "[workspace.package]\nversion = \"0.1.0\"\n";
    for bad in ["1.0.0-01", "1.0.0-rc.01", "1.0.0-00.1", "01.0.0", "1.0.0+meta"] {
        assert!(version::rewrite(&cfg, sample, bad).is_err(), "{bad}");
    }
    for ok in ["1.0.0-0", "1.0.0-rc.0", "1.0.0-01a", "1.0.0-01-2"] {
        let updated = version::rewrite(&cfg, sample, ok).unwrap();
        assert_eq!(version::read(&updated).unwrap(), ok);
    }
    let crlf = sample.replace('\n', "\r\n");
    assert_eq!(version::rewrite(&cfg, &crlf, "2.0.0").unwrap(), crlf.replace("0.1.0", "2.0.0"));
}

#[test]
fn ico_stdout_and_bytes_match_golden_and_committed_icon_is_valid() {
    let temp = Temp::new();
    for size in [16_u32, 32, 48, 256] {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&size.to_be_bytes());
        png.extend_from_slice(&size.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 1, 2, 3, 4]);
        temp.write(format!("{size}.png"), png);
    }
    assert_output(xtask_cmd(&["ico", "out.ico", "16.png", "32.png", "48.png", "256.png"], &temp.0), include_bytes!("golden/ico.stdout"));
    let actual: String = std::fs::read(temp.0.join("out.ico")).unwrap().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(actual, include_str!("golden/ico.hex").trim_end());
    assert!(ico::pack(&[]).is_err());

    let path = repo_root().join("assets/app-icon/photocraft.ico");
    if let Ok(ico_bytes) = std::fs::read(&path) {
        assert_eq!(&ico_bytes[..4], &[0, 0, 1, 0]);
        let n = u16::from_le_bytes([ico_bytes[4], ico_bytes[5]]) as usize;
        assert!(n >= 5);
        for i in 0..n {
            let e = &ico_bytes[6 + i * 16..6 + (i + 1) * 16];
            let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
            let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
            assert!(ico::png_size(&ico_bytes[off..off + len]).is_ok());
        }
    }
}

#[test]
fn sha256_matches_known_vectors() {
    let temp = Temp::new();
    for (idx, (bytes, expected)) in [
        (b"".as_slice(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        (b"abc".as_slice(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        (b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq".as_slice(), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"),
        (&[b'a'; 1000], "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"),
    ]
    .into_iter()
    .enumerate()
    {
        let path = temp.write(format!("v{idx}.bin"), bytes);
        assert_eq!(craft_xtask::sha256_hex(&path).unwrap(), expected);
    }
}

#[test]
fn layers_policy_enforces_photocraft_rules() {
    let cfg: layers::Config = toml::from_str(
        r#"
layers = [
  ["L0", ["photocraft-geom", "photocraft-cms", "photocraft-color", "photocraft-raster", "photocraft-psd", "photocraft-codecs", "photocraft-heif", "photocraft-ipp", "photocraft-raw", "photocraft-affinity", "photocraft-adobe-assets", "photocraft-tablet"]],
  ["L1", ["photocraft-doc"]],
  ["L2", ["photocraft-ops", "photocraft-paint", "photocraft-algo", "photocraft-text", "photocraft-vector"]],
  ["L3", ["photocraft-compose", "photocraft-gpu", "photocraft-format"]],
  ["L4", ["photocraft-io", "photocraft-tools", "photocraft-viewport", "photocraft-ml", "photocraft-plugins"]],
  ["L5", ["photocraft-engine"]],
  ["L6", ["photocraft-ui-egui", "photocraft-automation", "photocraft-platform", "photocraft-testkit"]],
]
exempt = ["photocraft", "photocraft-cli", "photocraft-web", "photocraft-web-boot", "xtask"]
standalone = ["photocraft-psd", "photocraft-codecs", "photocraft-heif", "photocraft-ipp", "photocraft-raw", "photocraft-affinity", "photocraft-adobe-assets", "photocraft-tablet"]
testkits = ["photocraft-testkit"]
allowed_edges = [
  ["photocraft-cms", "photocraft-geom"],
  ["photocraft-color", "photocraft-geom"],
  ["photocraft-color", "photocraft-cms"],
  ["photocraft-raster", "photocraft-geom"],
  ["photocraft-raster", "photocraft-cms"],
  ["photocraft-raster", "photocraft-color"],
  ["photocraft-gpu", "photocraft-compose"],
  ["photocraft-codecs", "photocraft-heif"],
]
[[external_rules]]
dependencies = ["egui", "eframe", "winit", "egui_kittest", "craft-ui", "rfd", "bevy*"]
allowed_from = ["photocraft-ui-egui", "photocraft-automation", "photocraft-platform", "photocraft-testkit"]
"#,
    )
    .unwrap();
    let edge = |from: &str, to: &str, kind: layers::DepKind, workspace: bool| layers::Crate {
        name: from.into(),
        deps: vec![layers::Dep { name: to.into(), kind, workspace }],
    };
    assert!(layers::check(&cfg, &[edge("photocraft-codecs", "photocraft-heif", layers::DepKind::Normal, true)]).unwrap().is_empty());
    assert!(layers::check(&cfg, &[edge("photocraft-raster", "photocraft-color", layers::DepKind::Normal, true)]).unwrap().is_empty());
    assert!(layers::check(&cfg, &[edge("photocraft-raster", "photocraft-testkit", layers::DepKind::Dev, true)]).unwrap().is_empty());
    for bad in [
        edge("photocraft-heif", "photocraft-codecs", layers::DepKind::Normal, true),
        edge("photocraft-doc", "photocraft-engine", layers::DepKind::Normal, true),
        edge("photocraft-ops", "photocraft-algo", layers::DepKind::Normal, true),
        edge("photocraft-raster", "photocraft-testkit", layers::DepKind::Normal, true),
        edge("photocraft-engine", "egui", layers::DepKind::Normal, false),
        layers::Crate { name: "photocraft-mystery".into(), deps: Vec::new() },
    ] {
        assert!(!layers::check(&cfg, &[bad]).unwrap().is_empty());
    }
}
