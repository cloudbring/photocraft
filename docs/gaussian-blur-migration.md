# Gaussian blur variance and shared core migration

PhotoCraft's Gaussian Blur now uses `craft-blur`'s `Quality::Recursive` plan.
The previous implementation exceeds 1% variance error at four of the seven
requested radii, so retaining it is not an accuracy-preserving option (#2).
Motion Blur's FFT implementation, Box Blur, the other blur kinds, and the
Gaussian helper used by other effects are unchanged.

## Impulse response measurements

The reference is main at `327c4cc82e737a3e0af7c147a477126d77525495`.
Measurements use a centered, unit, grayscale F32 2D impulse, zero background,
and a square with half-width `ceil(8 * sigma) + 8`. Accumulate mass and both
second moments in f64; report `(moment_x + moment_y) / (2 * mass)` and relative
error against `sigma²`. No alpha threshold or byte quantization is applied.

| Sigma | Main variance | Main error | Shared core error |
| ---: | ---: | ---: | ---: |
| 0.5 | 0.250000086 | +0.000034% | -0.000002% |
| 1 | 0.995911976 | -0.408802% | +0.000001% |
| 2 | 3.951262564 | -1.218436% | +0.000003% |
| 4 | 15.702566176 | -1.858961% | +0.000002% |
| 8 | 61.333668900 | -4.166142% | -0.033086% |
| 16 | 250.666921180 | -2.083234% | -0.111031% |
| 64 | 4074.676676906 | -0.520589% | -0.251645% |

Main uses variance-calibrated sampled kernels below 1 px, uncalibrated
3-sigma sampled kernels through 4 px, and integer three-box cascades above
4 px. The old approximate 0.4% variance claim does not hold for these radii.
The recursive response has infinite support; the fixed measurement window
omits some tails. Its measured mass remains within 0.0001 of one and its
measured variance error stays below 1% at every requested radius.

Reproduce the accuracy gate with:

```sh
cargo test -p photocraft-algo --test it gaussian_variance -- --nocapture
```

## Integration contract

- The dependency is the requested temporary branch
  `https://github.com/cloudbring/craft-libs`, `algo/2-craft-blur`, locked at
  `75bb40cf0c06b7e046874f389376acbfd86915ab`. The manifest TODO requires
  repointing to an immutable revision after the shared branch lands.
- `Quality::Recursive` selects a calibrated sampled kernel below 8 px, the
  recursive core from 8 through 65,536 px, and a sampled fallback above that.
  PhotoCraft retains its existing identity cutoff below 0.05 px.
- Sampled kernels retain PhotoCraft's separable convolution. Recursive passes
  retain its row batching, 64-pixel column strips, bounded output bands, rayon,
  tile writing, selection mixing, and progress reporting. Complete source lines
  are read because a finite tile halo cannot represent recursive support.
- The shared core supports `Edge::Clamp` on premultiplied floats. PhotoCraft
  premultiplies before both axes and unpremultiplies afterward, preserving its
  alpha-weighted edge repeat. Explicit output beyond the extent repeats source
  pixels **before** filtering, not already-filtered output. Without an explicit
  extent, a default-pixel border encloses content and output. The existing
  finite, 3-sigma layer output footprint is retained separately from reads.
- One through four channels use the packed shared API. More channels use
  per-channel planes, including CMYKA. Tests cover U8, U16, F32, grayscale, RGB,
  CMYK, multichannel, alpha, several tile sizes, ROI reads, hidden colors, and
  edge repeat. Shared cancellation checkpoints call PhotoCraft's `Interrupt`;
  cancellation returns no partial surface and never mutates the input.
- `Separable::Recursive` exposes actual shared coefficients instead of falsely
  advertising box passes. Its reach sentinel requires reading the entire
  extent; downstream device implementations must handle the new variant.

## Changed fixtures and processing compatibility

There are no tracked Gaussian golden images to bless. The golden-image API
and `xtask bless` mentioned in `docs/architecture.md` are not implemented in
`photocraft-testkit` or `xtask`; the snapshot section of `docs/development.md`
instead describes reviewing generated images. This migration adds a
deterministic numerical fixture and a frozen main-algorithm oracle, rather
than inventing a bless command or silently updating image files.

All nine RGBA32F fixtures deliberately change. Each uses a 96 by 64 image
containing a hard color step, partial transparency, and hidden zero-alpha
color, with repeated edges. Deltas are maximum absolute component differences
against the main algorithm, in straight and premultiplied RGBA respectively:

| Sigma | Straight maximum delta | Premultiplied maximum delta |
| ---: | ---: | ---: |
| 0.3 | 1.000000000 | 0.000030816 |
| 0.8 | 0.000000238 | 0.000000179 |
| 1 | 0.001743972 | 0.000774145 |
| 2 | 0.004890800 | 0.002630174 |
| 3 | 0.006797016 | 0.003575206 |
| 4 | 0.008478940 | 0.004105628 |
| 8 | 0.035242677 | 0.037572801 |
| 16 | 0.042386472 | 0.042386472 |
| 64 | 0.026779890 | 0.027556300 |

The straight maximum of 1 at sigma 0.3 is hidden RGB at effectively zero
alpha when sampled support changes; it is not an opaque one-level image
change. The maximum premultiplied RGBA delta across the fixtures is
0.042386472. Reproduce all rows with
`cargo test -p photocraft-algo --lib migration_fixture_deltas -- --nocapture`.

There is no documented or implemented per-filter processing-version selector
in the current document model. `SmartFilter` stores the command and parameters,
blend, opacity, and visibility, but no algorithm revision. Accordingly this
change does not invent a document schema/version bump: existing Gaussian smart
filters re-evaluate with the corrected algorithm, while already-rasterized
pixels remain unchanged. Preserving legacy smart-filter appearance would need
a separately designed versioned-processing mechanism.

## Criterion before and after

The existing 2048-square RGBA8 workloads and ten-sample configuration are
unchanged. Both measurements use rustup stable rustc 1.99.0, the same worktree
and `CARGO_TARGET_DIR=$PWD/target-2`, sequentially with no other local gate
running. The main source was restored temporarily for the baseline and the
migration restored before the second run; no branch or commit was changed.
Earlier Homebrew-compiler pilot timings are excluded from this comparison.

| Sigma | Main estimate and 95% interval in ms | Migrated estimate and 95% interval in ms | Criterion verdict |
| ---: | ---: | ---: | --- |
| 0.3 | 11.298 [10.719, 12.301] | 7.2373 [7.1964, 7.2898] | Improved, p < 0.05 |
| 0.8 | 9.6495 [9.5377, 9.7575] | 9.5349 [9.3626, 9.7103] | No significant change, p = 0.30 |
| 3 | 15.913 [15.288, 16.740] | 16.277 [15.675, 16.902] | No significant change, p = 0.51 |

These existing workloads exercise sampled kernels, not the recursive regime.
The migration is justified by accuracy rather than a claim of large-radius
speedup. The commands were:

```sh
export CARGO_TARGET_DIR="$PWD/target-2"
export RUSTC="$(rustup which rustc)"
export PATH="$(dirname "$(rustup which cargo)"):$PATH"
rustup run stable cargo bench -p photocraft-algo --bench blur -- --save-baseline main-rustup
rustup run stable cargo bench -p photocraft-algo --bench blur -- --baseline main-rustup
```

The explicit compiler path prevents a Homebrew `rustc` earlier on PATH from
being mixed with rustup's WebAssembly standard library. `target-2` is removed
after validation; the numerical results are retained here.

## Validation results

All commands use the rustup compiler and `target-2` configuration above.

- `cargo test -p photocraft-algo`: 761 unit tests and 14 integration tests
  passed, zero failures, 26 pre-existing ignored tests; zero doctests.
- `cargo clippy -p photocraft-algo --all-targets -- -D warnings`: passed.
- `cargo fmt --all --check` and `git diff --check`: passed.
- `cargo xtask layers`: 31 crates, zero violations.
- `cargo xtask wasm`: all 24 checked crates and
  `photocraft-codecs --features heif` passed.
- `cargo xtask perf --quick`: eight benches, 79 scenario rows, zero failures.
  Its saved regression baseline is for an M4 Pro; this host is an M6, so the
  harness correctly skipped cross-machine regression comparisons. This is not
  evidence of a performance improvement over that saved baseline.
- The variance and nine-fixture delta commands also passed individually.
  The dedicated outside-extent test covers edge extension and preservation
  of the layer output footprint.
