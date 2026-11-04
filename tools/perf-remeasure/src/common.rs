//! Shared helpers: summary rows and JSON output. Not a benchmark; see main.rs.
use craft_bench::report::Report;
use serde_json::{json, Value};

/// Speedup is old/new = 1/(new/old ratio); CI endpoints swap.
pub fn speedup(r: &craft_bench::report::Ratio) -> Value {
    json!({ "point": 1.0 / r.point, "ci_lo": 1.0 / r.ci_hi, "ci_hi": 1.0 / r.ci_lo })
}

pub fn summarize(rep: &Report) -> Value {
    let wall = speedup(&rep.aggregate.wall);
    let cpu = rep.aggregate.cpu.as_ref().map(speedup);
    let lo = wall["ci_lo"].as_f64().unwrap_or(0.0);
    let hi = wall["ci_hi"].as_f64().unwrap_or(0.0);
    let faster = lo > 1.0;
    json!({
        "inputs": rep.aggregate.inputs,
        "wall_speedup_geomean": wall,
        "cpu_speedup_geomean": cpu,
        "ci_excludes_1": faster || hi < 1.0,
        "direction": if faster {"faster"} else if hi < 1.0 {"slower"} else {"inconclusive"},
        "publishable": rep.publishable,
        "contended": rep.contended,
        "load_before": rep.guard.load_before,
        "load_after": rep.guard.load_after,
        "logical_cores": rep.guard.logical_cores,
    })
}

pub fn write(out_dir: &str, label: &str, rep: &Report, meta: Value) {
    std::fs::create_dir_all(out_dir).expect("mkdir");
    let doc = json!({
        "label": label,
        "meta": meta,
        "summary": summarize(rep),
        "craft_bench_report": serde_json::from_str::<Value>(&rep.to_json()).expect("report json"),
    });
    let path = format!("{out_dir}/{label}.json");
    std::fs::write(&path, serde_json::to_string_pretty(&doc).expect("ser")).expect("write");
    println!("== {label}\n{}\nsummary: {}\nwrote {path}", rep.to_markdown().lines().take(6).collect::<Vec<_>>().join("\n"), doc["summary"]);
}

pub fn load_photos(dir: &str) -> Vec<(String, usize, usize, Vec<[f32; 4]>)> {
    let mut paths: Vec<_> = std::fs::read_dir(dir).expect("photo dir").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jpg")).collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|p| {
            let img = image::open(&p).ok()?.to_rgb8();
            let (w, h) = (img.width() as usize, img.height() as usize);
            let px = img.pixels().map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]).collect();
            Some((p.file_stem()?.to_string_lossy().into_owned(), w, h, px))
        })
        .collect()
}

pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}
pub fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

pub fn lcg(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state >> 33
}
