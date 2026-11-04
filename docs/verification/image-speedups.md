# Image speedup remeasurement: photocraft (artcraft-algorithms#190, claims C02, C03, C08)

Measured 2026-11-04 (US Eastern evening) on Apple M6, 12 logical cores, 32 GiB, macOS (Darwin 27.0.0),
rustc 1.99.0, release profile (thin LTO, 1 codegen unit). Harness: `tools/perf-remeasure/`, a standalone
workspace using craft-bench (cloudbring/craft-libs rev 94c6d76e5bc2c936d3db5ce5e19d70213e8390c0).
Raw per-run data, machine fingerprint and both revs: `docs/verification/data/*.json` (field `craft_bench_report`).

## Method

- OLD = storytold/photocraft `327c4cc82e737a3e0af7c147a477126d77525495` (merge-base of every source branch with origin/main),
  linked as renamed git deps (`old-algo`, `old-psd`). NEW = this checkout by path.
- NEW revs: C02/C03 = `03f2995df4605477bd5a276baeded1ee29239b09` (algo/158-hdr-schur, stacked on algo/159-median-select
  `d61722faf749669cbfdcfabd7e63601161530e73`, so it contains both). C08 = `a39d2fa05518b9487918e7f5843a15ebe1554409`
  (algo/155-psd-packbits-parallel). That branch only touches `crates/psd`, so for the C08 run `crates/psd` and `Cargo.lock` were
  temporarily checked out from it (`git checkout a39d2fa05 -- crates/psd Cargo.lock`) and restored afterwards; the committed tree keeps the
  158 base. To reproduce C08 repeat that checkout first.
- Paired ABBA schedule, 20 timed runs per side per input (after 2 warm-up runs), 2000 bootstrap resamples, seeded. Speedup = old/new;
  the CI is the bootstrap 95% CI of the median ratio (geometric mean across inputs for multi-input runs).
- Output equivalence is checked before timing counts; inputs that differ are excluded and listed in the JSON (`meta.excluded`). None differed.
  HDR curves: max abs difference <= 1e-6 (observed max 1.7e-8 on real photos, 2.4e-12 on synthetic; the ticket quoted 4.3e-12 on its own fixture); medians and PSD planes: bit-identical.
- Load: craft-bench marks a run contended when the 1-minute load exceeds 0.5 per core (6.0). Load was about 4 to 5 throughout
  (other processes on this shared Mac), below the guard but not an idle machine. All runs are flagged publishable by the harness;
  treat the third digit with that caveat.

## Inputs

- Photos (C02, C03): 52 distinct real JPEGs extracted from the office corpus (`word/media/*.jpeg` >= 40 KB, deduplicated by sha256; manifest
  with source docx in the corpus tooling dir `worktrees/perf-inputs/office-jpegs/MANIFEST.tsv`). Lightcraft's raw corpus is not on this
  machine, so no camera raws. No bracketed exposure sets exist in the corpus: the middle exposure is the real photo and the +-2 stop exposures are
  simulated from its linear luminance through a gamma 2.2 response. The images are real, the bracket is synthetic.
- PSD (C08): photocraft's pinned corpora fetched with `cargo xtask corpus --photoshop --psd-tools --psd` into the gitignored `corpus/`
  (111 MB total, below the 2 GB limit; the full sets, not a sample): 737 files, 3 fail identically on both sides (excluded), 734 decoded.
  Operation timed: `PsdFile::from_bytes` + `decode_merged` + decode of every layer channel. Split by decoded size because the parallel path only
  engages at >= 1 MiB of decoded plane data.
- Synthetic (as the original claims): HDR 3 exposures at 160/640 samples; 1M f32 median (the original criterion form, OLD sort vendored from
  `crates/algo/benches/robust.rs@d61722faf`); 8192x8192x4 RLE `decode_planes`.

## Results

| run | inputs | wall speedup (median ratio old/new) [95% CI] | CPU-time speedup [95% CI] | load before/after (limit) | publishable |
|---|---|---|---|---|---|
| C02-hdr-real-160 | 52 | 1.48x [1.48, 1.48] | 1.48x [1.48, 1.48] | 4.24 / 4.22 (6.0) | yes |
| C02-hdr-real-640 | 52 | 23.23x [23.20, 23.25] | 23.19x [23.17, 23.21] | 4.22 / 4.77 (6.0) | yes |
| C02-hdr-synthetic-160 | 1 | 1.77x [1.76, 1.79] | 1.77x [1.76, 1.78] | 4.77 / 4.77 (6.0) | yes |
| C02-hdr-synthetic-640 | 1 | 27.29x [27.09, 27.36] | 27.26x [27.08, 27.29] | 4.77 / 4.62 (6.0) | yes |
| C03-median-estimate_exposures-real | 52 | 1.24x [1.23, 1.24] | 1.24x [1.23, 1.24] | 4.62 / 4.81 (6.0) | yes |
| C03-median-mtb_offset-real | 52 | 2.24x [2.24, 2.24] | 2.24x [2.24, 2.24] | 4.81 / 4.31 (6.0) | yes |
| C03-median-synthetic-1M | 1 | 16.17x [16.07, 16.26] | 16.14x [16.04, 16.23] | 4.31 / 4.31 (6.0) | yes |
| C08-psd-real-large | 57 | 1.00x [0.99, 1.01] | 0.46x [0.41, 0.47] | 4.05 / 4.05 (6.0) | yes |
| C08-psd-real-small | 677 | 0.96x [0.95, 0.96] | 0.95x [0.95, 0.96] | 4.05 / 4.05 (6.0) | yes |
| C08-psd-synthetic-8k | 1 | 6.68x [6.42, 6.88] | 0.76x [0.74, 0.78] | 4.05 / 4.61 (6.0) | yes |

CPU time is process CPU time over all threads. Mean sample count actually obtained from real photos: 155.7 (requested 160) and 619.9 (requested 640).

## Verdicts

| claim | claimed | measured wall speedup [95% CI] | verdict |
|---|---|---|---|
| C02 HDR calibration | 31x at 640 samples (1.7x at 160) | real photos 23.2x [23.2, 23.3] at 640, 1.48x at 160; synthetic 27.3x [27.1, 27.4] at 640, 1.77x at 160 | **partially**: large, real, output-equal (max curve diff within 1e-6); but the CI excludes 31x. The 160-sample figure (1.7x) reproduces on synthetic input. |
| C03 median filters | 16.9x (1M samples, sort vs select) | synthetic 1M: 16.17x [16.07, 16.26]; real call sites: `estimate_exposures` 1.24x [1.23, 1.24], `mtb_offset` 2.24x [2.24, 2.24] | **partially**: the kernel speedup reproduces within 5% (CI excludes 16.9x narrowly); the speedup of the functions users call on real photos is 1.2x to 2.2x, not 16.9x. Outputs bit-identical. |
| C08 PSD decode | 3.6x (8K x 8K x 4, 12 threads, 67.0 ms to 18.7 ms) | synthetic 6.68x [6.42, 6.88]; real corpus, 677 files < 1 MiB decoded: 0.96x [0.95, 0.96]; 57 files >= 1 MiB: 1.00x [0.99, 1.01] | **partially**: synthetic reproduces and exceeds 3.6x here. No corpus file benefits (largest decode is about 2 ms, highly compressible); small files are 4% slower (row-table validation). Process CPU time is 1.3x to 2.2x higher (rayon), so the gain is wall-clock only. |

## Caveats

- Real-photo HDR and median inputs use simulated brackets (see Inputs).
- Machine was lightly loaded by other work (load 4 to 5 of 12), not idle.
- The wall-clock PSD gain depends on core count (12 here).
