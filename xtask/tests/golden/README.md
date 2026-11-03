# Legacy xtask golden fixtures

Captured on 2026-11-03 from PhotoCraft `327c4cc82` before deleting the legacy
`ico.rs`, `layers.rs`, `sha256.rs`, `stats.rs`, and `version.rs` modules.

- `layers.stdout`: output of `cargo xtask layers` on the workspace.
- `stats.stdout`: output of `cargo xtask stats` on the untouched workspace.
- `version.stdout`: output of `cargo xtask version`.
- `version-set.stdout`, `version-after.toml`: output and rewritten root manifest
  for `version set 1.2.3-rc.1`.
- `ico.stdout`, `ico.hex`: output and packed bytes for `ico out.ico 16.png 32.png 48.png 256.png`
  on synthetic 16/32/48/256 px IHDR PNGs.
