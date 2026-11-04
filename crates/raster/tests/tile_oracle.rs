//! #188: OLD (BTreeMap tiles) vs NEW (`craft-tiledir::SparseTileDir`) tile storage.
//!
//! OLD is `photocraft-raster` at 327c4cc82e737a3e0af7c147a477126d77525495 (merge-base of
//! `algo/35-persistent-tile-directory` with `origin/main`), linked as `photocraft-raster-old`
//! (with its own `-geom-old` and `-color-old`). Both are driven by one op script through the same
//! macro-generated driver, and after every op the observable state is compared: tile set, tile
//! bytes, sharing pattern, bounds, region/pixel/row reads, plus the undo and redo stacks.
//!
//! Undo and redo are modelled the way `photocraft-ops` History does it: a snapshot is an O(1)
//! `Surface::clone`, undo/redo swap whole surfaces; `take_tiles`/`put_tiles` is the tile-level
//! hand-off used by parallel edits.

use proptest::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
enum Op {
    /// Set: write one pixel.
    Set {
        x: i32,
        y: i32,
        px: [f32; 4],
    },
    /// Set: write a region of pseudo-random samples.
    Write {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        seed: u32,
    },
    /// Flood: fill a rectangle (full tiles are shared).
    Flood {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        px: [f32; 4],
    },
    /// Clear: take the tiles of a rectangle out and drop them.
    ClearTiles {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    /// Clear: fill with the default pixel (tiles stay allocated).
    ClearToDefault {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    /// Edit a tile's bytes in place (copy-on-write when shared).
    TileByte {
        tx: i32,
        ty: i32,
        off: usize,
        val: u8,
    },
    TryTileByte {
        tx: i32,
        ty: i32,
        off: usize,
        val: u8,
    },
    Prune,
    ShareUniform,
    Translate {
        dx: i32,
        dy: i32,
    },
    /// History: push a snapshot (clone) and clear redo.
    Snapshot,
    Undo,
    Redo,
    /// Keep a clone aside to check later edits don't leak into it.
    KeepClone,
    /// Take tiles of a rect out, to be put back later (possibly after other edits).
    Hold {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    Restore,
}

fn px() -> impl Strategy<Value = [f32; 4]> {
    // Multiples of 1/8 and a few extremes, so u8/u16 encoding is exercised exactly and not.
    let v = prop_oneof![(0u8..=8).prop_map(|n| f32::from(n) / 8.0), Just(0.3f32), Just(0.999f32), Just(1.5f32), Just(-0.25f32)];
    [v.clone(), v.clone(), v.clone(), v]
}

fn coord() -> impl Strategy<Value = i32> {
    prop_oneof![-700i32..700, Just(-256), Just(-1), Just(0), Just(255), Just(256), Just(511), Just(512)]
}

fn op() -> impl Strategy<Value = Op> {
    let rect = || (coord(), coord(), 0u32..700, 0u32..700);
    let small = || (coord(), coord(), 0u32..120, 0u32..120);
    prop_oneof![
        3 => (coord(), coord(), px()).prop_map(|(x, y, px)| Op::Set { x, y, px }),
        2 => (small(), any::<u32>()).prop_map(|((x, y, w, h), seed)| Op::Write { x, y, w, h, seed }),
        3 => (rect(), px()).prop_map(|((x, y, w, h), px)| Op::Flood { x, y, w, h, px }),
        1 => rect().prop_map(|(x, y, w, h)| Op::ClearTiles { x, y, w, h }),
        1 => rect().prop_map(|(x, y, w, h)| Op::ClearToDefault { x, y, w, h }),
        2 => (-3i32..3, -3i32..3, 0usize..4096, any::<u8>()).prop_map(|(tx, ty, off, val)| Op::TileByte { tx, ty, off, val }),
        1 => (-3i32..3, -3i32..3, 0usize..4096, any::<u8>()).prop_map(|(tx, ty, off, val)| Op::TryTileByte { tx, ty, off, val }),
        1 => Just(Op::Prune),
        1 => Just(Op::ShareUniform),
        1 => (-600i32..600, -600i32..600).prop_map(|(dx, dy)| Op::Translate { dx, dy }),
        1 => prop_oneof![Just(Op::Translate { dx: 256, dy: -512 }), Just(Op::Translate { dx: -256, dy: 0 })],
        2 => Just(Op::Snapshot),
        2 => Just(Op::Undo),
        2 => Just(Op::Redo),
        1 => Just(Op::KeepClone),
        1 => rect().prop_map(|(x, y, w, h)| Op::Hold { x, y, w, h }),
        1 => Just(Op::Restore),
    ]
}

/// What a surface shows, as plain data (no types of either crate).
#[derive(Clone, Debug, PartialEq)]
struct View {
    /// Tiles in the order `tiles()` yields them.
    order: Vec<(i32, i32)>,
    /// Sorted by coordinate: (tx, ty, bytes digest, sharing class).
    tiles: Vec<(i32, i32, u64, usize)>,
    tile_count: usize,
    distinct_allocations: usize,
    content_bounds: (i32, i32, i32, i32),
    tile_bounds: (i32, i32, i32, i32),
    reads: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq)]
struct Step {
    cur: View,
    /// Digest per surface on the undo and redo stacks, and per kept clone.
    undo: Vec<u64>,
    redo: Vec<u64>,
    kept: Vec<u64>,
    held: usize,
}

fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

fn samples(seed: u32, n: usize) -> Vec<f32> {
    let mut s = u64::from(seed) | 1;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 40) % 9) as f32 / 8.0
        })
        .collect()
}

/// Reads probing every read path, on rectangles crossing tile borders and the origin.
const PROBES: [(i32, i32, u32, u32); 5] = [(-300, -300, 140, 90), (-5, -5, 20, 20), (250, 250, 12, 12), (500, -270, 40, 60), (-700, 600, 30, 30)];

macro_rules! side {
    ($m:ident, $raster:ident, $geom:ident, $color:ident) => {
        mod $m {
            use super::*;
            use $color::PixelFormat;
            use $geom::{Rect, TileCoord};
            use $raster::{Surface, Tile, TileInterner};

            pub const FORMATS: [PixelFormat; 4] = [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::GRAY8, PixelFormat::RGBA32F];

            fn rect(x: i32, y: i32, w: u32, h: u32) -> Rect {
                Rect::from_xywh(x, y, w, h)
            }

            fn bytes_digest(s: &Surface) -> u64 {
                fnv(s.tiles().flat_map(|(c, t)| {
                    let mut v = c.tx.to_le_bytes().to_vec();
                    v.extend_from_slice(&c.ty.to_le_bytes());
                    v.extend_from_slice(t.bytes());
                    v
                }))
            }

            fn view(s: &Surface) -> View {
                let order: Vec<(i32, i32)> = s.tiles().map(|(c, _)| (c.tx, c.ty)).collect();
                let mut ptrs: HashMap<usize, usize> = HashMap::new();
                let mut tiles: Vec<(i32, i32, u64, usize)> = s
                    .tiles()
                    .map(|(c, t)| {
                        let next = ptrs.len();
                        let class = *ptrs.entry(Arc::as_ptr(t) as usize).or_insert(next);
                        (c.tx, c.ty, fnv(t.bytes().iter().copied()), class)
                    })
                    .collect();
                tiles.sort();
                // Sharing classes in coordinate order, so the labelling doesn't depend on iteration order.
                let mut relabel: HashMap<usize, usize> = HashMap::new();
                for t in &mut tiles {
                    let next = relabel.len();
                    t.3 = *relabel.entry(t.3).or_insert(next);
                }
                let c = s.content_bounds();
                let b = s.tile_bounds();
                let n = s.channels();
                let mut reads = vec![];
                for &(x, y, w, h) in &PROBES {
                    let r = rect(x, y, w, h);
                    let region = s.read_region(r);
                    reads.push(fnv(region.iter().flat_map(|f| f.to_bits().to_le_bytes())));
                    reads.push(fnv(s.to_interleaved(r)));
                    reads.push(u64::from(s.has_tiles_in(r)));
                    let mut rgba8 = vec![[0u8; 4]; (w * h) as usize];
                    s.read_rgba8_into(r, &mut rgba8);
                    reads.push(fnv(rgba8.iter().flatten().copied()));
                    let mut rgba = vec![[0f32; 4]; (w * h) as usize];
                    s.read_rgba_into(r, &mut rgba);
                    reads.push(fnv(rgba.iter().flatten().flat_map(|f| f.to_bits().to_le_bytes())));
                    reads.push(s.try_read_region(r).map_or(0, |v| fnv(v.iter().flat_map(|f| f.to_bits().to_le_bytes()))));
                    let mut row = vec![0f32; 40];
                    s.sample_row_strided(y, x, 7, 0, &mut row);
                    reads.push(fnv(row.iter().flat_map(|f| f.to_bits().to_le_bytes())));
                    let p = s.pixel(x + 1, y + 1);
                    reads.push(fnv(p.iter().flat_map(|f| f.to_bits().to_le_bytes())));
                    reads.push(s.sample_channel(x, y, n - 1).to_bits().into());
                }
                View {
                    order,
                    distinct_allocations: ptrs.len(),
                    tile_count: s.tile_count(),
                    tiles,
                    content_bounds: (c.x0, c.y0, c.x1, c.y1),
                    tile_bounds: (b.x0, b.y0, b.x1, b.y1),
                    reads,
                }
            }

            pub fn run(format: usize, ops: &[Op]) -> Vec<Step> {
                let format = FORMATS[format];
                let n = format.channels();
                let mut cur = Surface::new(format);
                let (mut undo, mut redo, mut kept): (Vec<Surface>, Vec<Surface>, Vec<(Surface, u64)>) = (vec![], vec![], vec![]);
                let mut held: Vec<Vec<(TileCoord, Arc<Tile>)>> = vec![];
                let mut interner = TileInterner::default();
                let mut out = vec![];
                for op in ops {
                    match op {
                        Op::Set { x, y, px } => cur.write_pixel(*x, *y, &px[..n]),
                        Op::Write { x, y, w, h, seed } => {
                            let r = rect(*x, *y, *w, *h);
                            let data = samples(*seed, (*w as usize) * (*h as usize) * n);
                            if seed % 2 == 0 { cur.write_region(r, &data) } else { cur.try_write_region(r, &data).unwrap() }
                        }
                        Op::Flood { x, y, w, h, px } => cur.fill_rect(rect(*x, *y, *w, *h), &px[..n]),
                        Op::ClearTiles { x, y, w, h } => drop(cur.take_tiles(rect(*x, *y, *w, *h))),
                        Op::ClearToDefault { x, y, w, h } => {
                            let d = cur.default_pixel();
                            cur.fill_rect(rect(*x, *y, *w, *h), &d)
                        }
                        Op::TileByte { tx, ty, off, val } => {
                            let t = cur.tile_mut(TileCoord::new(*tx, *ty));
                            let len = t.bytes().len();
                            t.bytes_mut()[off % len] = *val;
                        }
                        Op::TryTileByte { tx, ty, off, val } => {
                            let t = cur.try_tile_mut(TileCoord::new(*tx, *ty)).unwrap();
                            let len = t.bytes().len();
                            t.bytes_mut()[off % len] = *val;
                        }
                        Op::Prune => cur.prune(),
                        Op::ShareUniform => {
                            let saved = cur.share_uniform_tiles(&mut interner);
                            out.push(Step {
                                cur: View { reads: vec![saved as u64], ..view(&Surface::new(format)) },
                                undo: vec![],
                                redo: vec![],
                                kept: vec![],
                                held: 0,
                            });
                        }
                        Op::Translate { dx, dy } => {
                            let content = cur.content_bounds();
                            cur = cur.translated(*dx, *dy, content);
                        }
                        Op::Snapshot => {
                            undo.push(cur.clone());
                            redo.clear();
                        }
                        Op::Undo => {
                            if let Some(prev) = undo.pop() {
                                redo.push(std::mem::replace(&mut cur, prev));
                            }
                        }
                        Op::Redo => {
                            if let Some(next) = redo.pop() {
                                undo.push(std::mem::replace(&mut cur, next));
                            }
                        }
                        Op::KeepClone => {
                            let c = cur.clone();
                            let d = bytes_digest(&c);
                            kept.push((c, d));
                        }
                        Op::Hold { x, y, w, h } => held.push(cur.take_tiles(rect(*x, *y, *w, *h))),
                        Op::Restore => {
                            if let Some(t) = held.pop() {
                                cur.put_tiles(t);
                            }
                        }
                    }
                    // Kept clones must be exactly as they were when taken.
                    for (c, d) in &kept {
                        assert_eq!(bytes_digest(c), *d, "a kept clone changed after {op:?}");
                    }
                    out.push(Step {
                        cur: view(&cur),
                        undo: undo.iter().map(bytes_digest).collect(),
                        redo: redo.iter().map(bytes_digest).collect(),
                        kept: kept.iter().map(|(c, _)| bytes_digest(c)).collect(),
                        held: held.iter().map(Vec::len).sum(),
                    });
                }
                out
            }
        }
    };
}

side!(old, photocraft_raster_old, photocraft_geom_old, photocraft_color_old);
side!(new, photocraft_raster, photocraft_geom, photocraft_color);

/// Compare two step logs, ignoring tile iteration order (reported by its own test).
fn same_modulo_order(a: &[Step], b: &[Step]) -> Result<(), String> {
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        let (mut x, mut y) = (x.clone(), y.clone());
        x.cur.order.clear();
        y.cur.order.clear();
        if x != y {
            return Err(format!("step {i}: old {x:?}\n new {y:?}"));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

    /// Random edit histories (set / write / flood / clear / tile edits / translate / snapshot /
    /// undo / redo / take+put) leave identical tile storage in every pixel format.
    #[test]
    fn random_edit_histories_read_identically(format in 0usize..4, ops in prop::collection::vec(op(), 1..40)) {
        let (a, b) = (old::run(format, &ops), new::run(format, &ops));
        prop_assert_eq!(a.len(), b.len());
        if let Err(e) = same_modulo_order(&a, &b) {
            return Err(TestCaseError::fail(e));
        }
    }
}

#[test]
fn flood_then_edit_shares_then_splits_like_before() {
    let ops = [
        Op::Flood { x: -512, y: -512, w: 1024, h: 1024, px: [0.5, 0.25, 0.75, 1.0] },
        Op::Snapshot,
        Op::TileByte { tx: 0, ty: 0, off: 17, val: 9 },
        Op::Undo,
        Op::Redo,
        Op::ClearTiles { x: 0, y: 0, w: 300, h: 300 },
        Op::Undo,
    ];
    for f in 0..4 {
        let (a, b) = (old::run(f, &ops), new::run(f, &ops));
        same_modulo_order(&a, &b).unwrap();
        // 4x4 fully covered tiles from one flood share one allocation.
        assert_eq!(a[0].cur.distinct_allocations, 1);
        assert_eq!(b[0].cur.distinct_allocations, 1);
    }
}

/// OLD iterated tiles in `TileCoord` order (a BTreeMap). Reports whether NEW does too: callers
/// that serialise or export tiles by iteration see a different order otherwise.
#[test]
fn tile_iteration_order() {
    let ops: Vec<Op> = (0..6).flat_map(|i| [Op::Set { x: (i * 391) % 1500 - 700, y: (i * 577) % 1500 - 700, px: [1.0; 4] }]).collect();
    let (a, b) = (old::run(0, &ops), new::run(0, &ops));
    let (oa, ob) = (&a.last().unwrap().cur.order, &b.last().unwrap().cur.order);
    let mut sorted = ob.clone();
    sorted.sort();
    assert_eq!(
        oa,
        &{
            let mut s = oa.clone();
            s.sort();
            s
        },
        "OLD iterates in TileCoord order"
    );
    assert_eq!(oa, &sorted, "NEW holds the same tiles");
    assert_eq!(oa, ob, "NEW iterates in the same order as OLD");
}

/// The comparison is sensitive: one extra pixel write on one side is caught.
#[test]
fn harness_detects_a_difference() {
    let ops = [Op::Flood { x: 0, y: 0, w: 600, h: 300, px: [1.0; 4] }, Op::Snapshot];
    let mut more = ops.to_vec();
    more.push(Op::Set { x: 3, y: 3, px: [0.0; 4] });
    let (a, b) = (old::run(0, &ops), new::run(0, &more));
    assert!(same_modulo_order(&a, &b[..a.len()]).is_ok());
    assert!(
        same_modulo_order(
            &old::run(0, &more),
            &new::run(0, &ops)[..a.len()].iter().cloned().chain(new::run(0, &[Op::Set { x: 4, y: 3, px: [0.0; 4] }])).collect::<Vec<_>>()
        )
        .is_err()
    );
    assert!(a.last().unwrap().cur.tile_count > 0);
}
