#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Surface, Tile, scratch};

/// The previous Surface storage and mutation algorithm, retained as a benchmark oracle.
#[derive(Clone)]
struct BTreeSurface {
    default_pixel: Box<[u8]>,
    tiles: BTreeMap<TileCoord, Arc<Tile>>,
}

trait EditSurface: Clone {
    fn edit(&mut self, coord: TileCoord, value: u8, dense: bool);
}

fn edit_bytes(tile: &mut Tile, value: u8, dense: bool) {
    if dense {
        tile.bytes_mut().fill(value);
    } else {
        tile.bytes_mut()[0] = value;
    }
}

impl EditSurface for BTreeSurface {
    fn edit(&mut self, coord: TileCoord, value: u8, dense: bool) {
        assert_eq!(self.default_pixel.len(), 1);
        edit_bytes(Arc::make_mut(self.tiles.get_mut(&coord).unwrap()), value, dense);
    }
}

impl EditSurface for Surface {
    fn edit(&mut self, coord: TileCoord, value: u8, dense: bool) {
        edit_bytes(self.tile_mut(coord), value, dense);
    }
}

fn seeded(side: i32, spacing: i32) -> Surface {
    let mut seed = Surface::new(PixelFormat::GRAY8);
    seed.fill_rect(Rect::new(0, 0, 256, 256), &[1.0]);
    let tile = seed.tiles().next().unwrap().1.clone();
    let mut surface = Surface::new(PixelFormat::GRAY8);
    surface.put_tiles((0..side).flat_map(|tx| (0..side).map(move |ty| TileCoord::new(tx * spacing, ty * spacing))).map(|coord| (coord, tile.clone())));
    surface
}

/// Setup, history-vector capacity and destruction are excluded. Pixel COW allocations
/// are included. No worker threads are used: the allocator measures the calling thread.
fn sample<Storage: EditSurface>(mut surface: Storage, dense: bool) -> (f64, u64) {
    scratch::release();
    let steps = if dense { 1 } else { 200 };
    let mut history = Vec::with_capacity(steps);
    let mut elapsed = Default::default();
    let allocations = allocation_counter::measure(|| {
        let start = Instant::now();
        for step in 0..steps {
            history.push(surface.clone());
            if dense {
                for tx in 0..32 {
                    for ty in 0..32 {
                        surface.edit(TileCoord::new(tx, ty), 127, true);
                    }
                }
            } else {
                let index = (step * 37) % 4096;
                surface.edit(TileCoord::new((index / 64) as i32 * 2, (index % 64) as i32 * 2), (step % 254) as u8, false);
            }
        }
        black_box(&surface);
        black_box(&history);
        elapsed = start.elapsed();
    });
    (elapsed.as_secs_f64() * 1000.0, allocations.count_total)
}

#[test]
fn surface_clone_allocations_do_not_grow_with_tile_count() {
    for side in [1, 4, 64] {
        let surface = seeded(side, 2);
        let allocations = allocation_counter::measure(|| {
            black_box(surface.clone());
        });
        assert_eq!(allocations.count_total, 1, "only the one-byte default pixel is copied, side={side}");
        assert_eq!(allocations.bytes_total, 1);
    }
}

#[test]
#[ignore = "comparative timing/allocation benchmark; run with --release --ignored --nocapture --test-threads=1"]
fn snapshot_edit_benchmark() {
    for dense in [false, true] {
        let (side, spacing) = if dense { (32, 1) } else { (64, 2) };
        let surface = seeded(side, spacing);
        let before = BTreeSurface { default_pixel: vec![0].into_boxed_slice(), tiles: surface.tiles().map(|(coord, tile)| (coord, tile.clone())).collect() };
        let mut old_samples = Vec::new();
        let mut new_samples = Vec::new();
        for round in 0..8 {
            let (old, new) = if round % 2 == 0 {
                (sample(before.clone(), dense), sample(surface.clone(), dense))
            } else {
                let new = sample(surface.clone(), dense);
                (sample(before.clone(), dense), new)
            };
            if round != 0 {
                old_samples.push(old);
                new_samples.push(new);
            }
        }
        old_samples.sort_by(|left, right| left.0.total_cmp(&right.0));
        new_samples.sort_by(|left, right| left.0.total_cmp(&right.0));
        assert!(old_samples.iter().all(|sample| sample.1 == old_samples[0].1));
        assert!(new_samples.iter().all(|sample| sample.1 == new_samples[0].1));
        let scenario = if dense { "1024 tiles / full-canvas edit" } else { "4096 tiles / 200 one-tile snapshots" };
        println!(
            "{scenario}: BTreeMap {:.3} ms / {} allocations; SparseTileDir {:.3} ms / {} allocations (median of 7)",
            old_samples[3].0, old_samples[3].1, new_samples[3].0, new_samples[3].1
        );
    }
}
