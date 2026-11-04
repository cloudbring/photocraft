//! Paired remeasurement of photocraft speedup claims (artcraft-algorithms#190).
//! Usage: perf-remeasure <hdr|median|psd> <out_dir> <photos_dir> <psd_root>
//! Env: OLD_REV, NEW_REV (full SHAs, recorded in the report), REPS (default 20).
mod common;
use common::*;
use craft_bench::{report::Revs, Config, Paired};
use serde_json::{json, Value};

fn cfg() -> Config {
    let reps = std::env::var("REPS").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    Config {
        repetitions: reps,
        revs: Revs { old: std::env::var("OLD_REV").ok(), new: std::env::var("NEW_REV").ok() },
        ..Config::default()
    }
}

/// Keep inputs whose old and new outputs agree; return the names of the rest with the reason.
fn keep_equal<I, O>(inputs: Vec<(String, I)>, old: impl Fn(&I) -> O, new: impl Fn(&I) -> O, eq: impl Fn(&O, &O) -> Result<(), String>) -> (Vec<(String, I)>, Vec<Value>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for (n, i) in inputs {
        match eq(&old(&i), &new(&i)) {
            Ok(()) => kept.push((n, i)),
            Err(e) => dropped.push(json!({"input": n, "reason": e})),
        }
    }
    (kept, dropped)
}

// ---------------------------------------------------------------- HDR calibration (C02)

#[derive(Clone)]
struct HdrIn {
    z: Vec<Vec<u8>>,
    log_dt: Vec<f64>,
}

const DT: [f64; 3] = [0.25, 1.0, 4.0];

fn code(e: f32) -> u8 {
    (255.0 * e.clamp(0.0, 1.0).powf(1.0 / 2.2)).round() as u8
}

fn hdr_from_photo(w: usize, h: usize, px: &[[f32; 4]], count: usize) -> HdrIn {
    // Middle exposure = the photo itself; the others are simulated from its linear luminance
    // through a gamma-2.2 camera response with one-stop*2 steps (no real bracketed set is in the corpus).
    let idx = old_algo::hdr::sample_pixels(w, h, px, count);
    let lum: Vec<f32> = idx.iter().map(|&i| {
        let p = px[i];
        0.2126 * srgb_to_linear(p[0]) + 0.7152 * srgb_to_linear(p[1]) + 0.0722 * srgb_to_linear(p[2])
    }).collect();
    HdrIn { z: DT.iter().map(|dt| lum.iter().map(|&y| code(y * *dt as f32)).collect()).collect(), log_dt: DT.iter().map(|d| d.ln()).collect() }
}

fn hdr_synth(n: usize, seed: u64) -> HdrIn {
    let mut s = seed;
    let lum: Vec<f32> = (0..n).map(|_| (lcg(&mut s) as f32 / (1u64 << 31) as f32).powi(3).max(1e-4)).collect();
    HdrIn { z: DT.iter().map(|dt| lum.iter().map(|&y| code(y * *dt as f32)).collect()).collect(), log_dt: DT.iter().map(|d| d.ln()).collect() }
}

fn curve_eq(a: &Vec<f64>, b: &Vec<f64>) -> Result<(), String> {
    if a.len() != b.len() {
        return Err("length".into());
    }
    let m = a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max);
    if m <= 1e-6 { Ok(()) } else { Err(format!("max abs diff {m:e} > 1e-6")) }
}

fn max_curve_diff(sets: &[(String, HdrIn)]) -> f64 {
    sets.iter().map(|(_, i)| {
        let a = old_algo::hdr::response_curve(&i.z, &i.log_dt, 100.0);
        let b = new_algo::hdr::response_curve(&i.z, &i.log_dt, 100.0);
        a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max)
    }).fold(0.0, f64::max)
}

fn run_hdr(out: &str, photos: &str) {
    let old = |i: &HdrIn| old_algo::hdr::response_curve(&i.z, &i.log_dt, 100.0);
    let new = |i: &HdrIn| new_algo::hdr::response_curve(&i.z, &i.log_dt, 100.0);
    let ph = load_photos(photos);
    for count in [160usize, 640] {
        let inputs: Vec<(String, HdrIn)> = ph.iter().map(|(n, w, h, px)| (format!("photo:{n}:{count}samples"), hdr_from_photo(*w, *h, px, count))).filter(|(_, i)| !i.z[0].is_empty()).collect();
        let md = max_curve_diff(&inputs);
        let (kept, dropped) = keep_equal(inputs, old, new, curve_eq);
        let rep = Paired::new(format!("photocraft response_curve real photos {count} samples, 3 simulated exposures")).old(old).new(new).inputs(kept.clone()).check(|a, b| curve_eq(a, b).is_ok()).run(&cfg()).expect("run");
        write(out, &format!("C02-hdr-real-{count}"), &rep, json!({"claim":"C02 31x at 640 samples","input":"real photos (office-corpus JPEGs), middle exposure = photo, +-2 stops simulated","n_inputs":kept.len(),"mean_actual_samples":kept.iter().map(|(_,i)| i.z[0].len()).sum::<usize>() as f64 / kept.len().max(1) as f64,"min_actual_samples":kept.iter().map(|(_,i)| i.z[0].len()).min(),"excluded":dropped,"max_abs_curve_diff_all_inputs":md,"tolerance":1e-6}));
    }
    for count in [160usize, 640] {
        let inputs = vec![(format!("synthetic:{count}samples"), hdr_synth(count, 7))];
        let md = max_curve_diff(&inputs);
        let rep = Paired::new(format!("photocraft response_curve synthetic {count} samples")).old(old).new(new).inputs(inputs).check(|a, b| curve_eq(a, b).is_ok()).run(&cfg()).expect("run");
        write(out, &format!("C02-hdr-synthetic-{count}"), &rep, json!({"claim":"C02 original synthetic form (3 exposures, 160/640 samples)","max_abs_curve_diff":md,"tolerance":1e-6}));
    }
}

// ---------------------------------------------------------------- medians (C03)

fn lin_scale(px: &[[f32; 4]], k: f32) -> Vec<[f32; 4]> {
    px.iter().map(|p| [linear_to_srgb(srgb_to_linear(p[0]) * k), linear_to_srgb(srgb_to_linear(p[1]) * k), linear_to_srgb(srgb_to_linear(p[2]) * k), 1.0]).collect()
}

/// OLD form of the median, vendored: crates/algo/benches/robust.rs "sort" arm at
/// d61722faf (artcraft-algorithms#159), which is the sort the replaced sites performed.
fn median_sort(v: &Vec<f32>) -> f32 {
    let mut values = v.clone();
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

fn run_median(out: &str, photos: &str) {
    let ph = load_photos(photos);
    // estimate_exposures: 3 exposures (x1, x2, x4 linear) per photo.
    let ex: Vec<(String, Vec<Vec<[f32; 4]>>)> = ph.iter().map(|(n, _, _, px)| (format!("photo:{n}"), vec![px.clone(), lin_scale(px, 2.0), lin_scale(px, 4.0)])).collect();
    let old = |i: &Vec<Vec<[f32; 4]>>| { let r: Vec<&[[f32; 4]]> = i.iter().map(|v| v.as_slice()).collect(); old_algo::hdr::estimate_exposures(&r) };
    let new = |i: &Vec<Vec<[f32; 4]>>| { let r: Vec<&[[f32; 4]]> = i.iter().map(|v| v.as_slice()).collect(); new_algo::hdr::estimate_exposures(&r) };
    let eq = |a: &Vec<f64>, b: &Vec<f64>| if a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits()) && a.len() == b.len() { Ok(()) } else { Err("not bit-identical".into()) };
    let (kept, dropped) = keep_equal(ex, old, new, eq);
    let rep = Paired::new("photocraft estimate_exposures (median site) real photos").old(old).new(new).inputs(kept.clone()).check(|a, b| eq(a, b).is_ok()).run(&cfg()).expect("run");
    write(out, "C03-median-estimate_exposures-real", &rep, json!({"claim":"C03 16.9x (median filters)","site":"hdr::estimate_exposures (sort -> select_nth over step_by(7) ratios)","n_inputs":kept.len(),"excluded":dropped,"equality":"bit-identical f64"}));

    // mtb_offset: grey reference vs same grey shifted by (3,-2), 4 levels.
    let mt: Vec<(String, (usize, usize, Vec<f32>, Vec<f32>))> = ph.iter().map(|(n, w, h, px)| {
        let g: Vec<f32> = px.iter().map(|p| 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]).collect();
        let mut s = vec![0.0f32; g.len()];
        for y in 0..*h { for x in 0..*w { let (sx, sy) = (x as i64 - 3, y as i64 + 2); if sx >= 0 && sy >= 0 && (sx as usize) < *w && (sy as usize) < *h { s[y * w + x] = g[sy as usize * w + sx as usize]; } } }
        (format!("photo:{n}"), (*w, *h, g, s))
    }).collect();
    let old = |i: &(usize, usize, Vec<f32>, Vec<f32>)| old_algo::hdr::mtb_offset(i.0, i.1, &i.2, &i.3, 4);
    let new = |i: &(usize, usize, Vec<f32>, Vec<f32>)| new_algo::hdr::mtb_offset(i.0, i.1, &i.2, &i.3, 4);
    let eq = |a: &(i32, i32), b: &(i32, i32)| if a == b { Ok(()) } else { Err(format!("{a:?} vs {b:?}")) };
    let (kept, dropped) = keep_equal(mt, old, new, eq);
    let rep = Paired::new("photocraft mtb_offset (bitmaps median) real photos").old(old).new(new).inputs(kept.clone()).check(|a, b| a == b).run(&cfg()).expect("run");
    write(out, "C03-median-mtb_offset-real", &rep, json!({"claim":"C03 16.9x","site":"hdr::mtb_offset bitmaps() median","shift":[3,-2],"levels":4,"n_inputs":kept.len(),"excluded":dropped,"equality":"offset tuple identical"}));

    // Synthetic: 1M f32 samples, the original criterion form (sort vs select).
    let mut s = 0x1234_5678u32;
    let samples: Vec<f32> = (0..1_000_000).map(|_| { s = s.wrapping_mul(1664525).wrapping_add(1013904223); f32::from_bits(0x3f80_0000 | (s >> 9)) }).collect();
    let rep = Paired::new("median_f32 1M synthetic: sort (vendored) vs select").old(median_sort).new(|v: &Vec<f32>| { let mut x = v.clone(); new_algo::robust::median_f32(&mut x).unwrap() })
        .inputs([("synthetic:1M f32".to_string(), samples)]).check(|a: &f32, b: &f32| a.to_bits() == b.to_bits()).run(&cfg()).expect("run");
    write(out, "C03-median-synthetic-1M", &rep, json!({"claim":"C03 original criterion form","note":"both arms include the Vec clone; OLD vendored from crates/algo/benches/robust.rs@d61722faf"}));
}

// ---------------------------------------------------------------- PSD decode (C08)

fn decode_old(b: &Vec<u8>) -> Result<Vec<Vec<u8>>, String> {
    let f = old_psd::PsdFile::from_bytes(b).map_err(|e| e.to_string())?;
    let mut v = vec![f.decode_merged().map_err(|e| e.to_string())?];
    for l in f.layers() { for c in &l.channels { v.push(l.decode_channel(c.id, f.header.depth, f.header.version).map_err(|e| e.to_string())?); } }
    Ok(v)
}
fn decode_new(b: &Vec<u8>) -> Result<Vec<Vec<u8>>, String> {
    let f = new_psd::PsdFile::from_bytes(b).map_err(|e| e.to_string())?;
    let mut v = vec![f.decode_merged().map_err(|e| e.to_string())?];
    for l in f.layers() { for c in &l.channels { v.push(l.decode_channel(c.id, f.header.depth, f.header.version).map_err(|e| e.to_string())?); } }
    Ok(v)
}
fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() { walk(&p, out) } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("psd") || x.eq_ignore_ascii_case("psb")) { out.push(p) }
    }
}

fn run_psd(out: &str, root: &str) {
    let mut paths = Vec::new();
    walk(std::path::Path::new(root), &mut paths);
    paths.sort();
    let mut small = Vec::new();
    let mut large = Vec::new();
    let mut dropped = Vec::new();
    let mut failed_both = 0usize;
    for p in &paths {
        let Ok(bytes) = std::fs::read(p) else { continue };
        let name = p.strip_prefix(root).unwrap_or(p).display().to_string();
        let (o, n) = (decode_old(&bytes), decode_new(&bytes));
        match (&o, &n) {
            (Ok(a), Ok(b)) if a == b => {
                let total: usize = a.iter().map(Vec::len).sum();
                if total >= 1 << 20 { large.push((name, bytes)) } else { small.push((name, bytes)) }
            }
            (Err(a), Err(b)) if a == b => failed_both += 1,
            _ => dropped.push(json!({"input": name, "reason": "old/new output or error differs"})),
        }
    }
    println!("psd files: {} small {} large(>=1MiB decoded) {} failed identically {} differ {}", paths.len(), small.len(), large.len(), failed_both, dropped.len());
    let meta = |what: &str, n: usize| json!({"claim":"C08 3.6x (synthetic 8Kx8Kx4 RLE, 12 threads)","input":what,"n_inputs":n,"files_found":paths.len(),"failed_identically_on_both_excluded":failed_both,"output_differs_excluded":dropped,"equality":"every decoded plane byte-identical, or identical error","operation":"PsdFile::from_bytes + decode_merged + decode every layer channel"});
    for (label, set) in [("small", small), ("large", large)] {
        if set.is_empty() { continue; }
        let n = set.len();
        let rep = Paired::new(format!("photocraft PSD decode real files ({label} = {} 1 MiB decoded)", if label == "small" { "<" } else { ">=" })).old(decode_old).new(decode_new).inputs(set).check(|a, b| a == b).run(&cfg()).expect("run");
        write(out, &format!("C08-psd-real-{label}"), &rep, meta(&format!("photocraft corpus psd/photoshop/psd-tools, {label}"), n));
    }
    // Synthetic: 8192x8192x4 RLE planes through decode_planes (the original criterion form).
    {
        use old_psd::compression as oc;
        let layout = oc::PlaneLayout { planes: 4, width: 8192, height: 8192, depth: 8, version: old_psd::Version::Psb };
        let pixels: Vec<u8> = (0..layout.planes * layout.width * layout.height).map(|i| ((i / 8) % 251) as u8).collect();
        let enc = oc::encode_planes(oc::Compression::Rle, &pixels, &layout).expect("enc");
        drop(pixels);
        let old = |e: &Vec<u8>| oc::decode_planes(oc::Compression::Rle, e, &layout).expect("old");
        let nl = new_psd::compression::PlaneLayout { planes: 4, width: 8192, height: 8192, depth: 8, version: new_psd::Version::Psb };
        let new = |e: &Vec<u8>| new_psd::compression::decode_planes(new_psd::compression::Compression::Rle, e, &nl).expect("new");
        let rep = Paired::new("photocraft decode_planes RLE synthetic 8192x8192x4").old(old).new(new).inputs([("synthetic:8k x 8k x 4 RLE".to_string(), enc)]).check(|a: &Vec<u8>, b: &Vec<u8>| a == b).run(&cfg()).expect("run");
        write(out, "C08-psd-synthetic-8k", &rep, json!({"claim":"C08 original synthetic form","equality":"byte-identical"}));
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (what, out, photos, psd) = (a[1].as_str(), a[2].as_str(), a.get(3).map_or("", |s| s), a.get(4).map_or("", |s| s));
    match what {
        "hdr" => run_hdr(out, photos),
        "median" => run_median(out, photos),
        "psd" => run_psd(out, psd),
        _ => eprintln!("hdr|median|psd"),
    }
}
