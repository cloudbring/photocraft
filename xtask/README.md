# PhotoCraft workspace tasks

Ticket #70 delegates reusable tooling to `craft-xtask` (`algo/integration`).
Only `xtask` depends on `craft-xtask`; runtime crates are unchanged.

| Task | Shared API | PhotoCraft policy |
|---|---|---|
| `version` | `version::run`, `version::compute` | `strict_prerelease: true` rejects numeric prerelease identifiers with leading zeroes |
| `stats` | `stats::run`, `stats::report` | `follow_symlinks: true` preserves legacy directory traversal |
| `ico` | `ico::run`, `ico::pack` | Default config (`root: "."`, square PNGs <= 256 px) |
| `layers` | `layers::compute`, `layers::from_metadata` | PhotoCraft L0–L6 table, standalone/testkit/exempt rules, `codecs -> heif` exception, UI external boundary, and local table formatting over `layers::Report` |
| SHA-256 | `craft_xtask::sha256_hex` | Streaming file hashing in `corpus.rs` and `pinned.rs` |

PhotoCraft-specific tasks (`corpus`, `corpus_pins`, `i18n_coverage`, `perf`,
`pinned`, `scorecard`, and the UI example `parity` runner) remain local.
