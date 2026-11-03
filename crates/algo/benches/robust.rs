use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use photocraft_algo::robust::median_f32;
use std::hint::black_box;

fn median_bench(c: &mut Criterion) {
    let mut state = 0x1234_5678u32;
    let samples: Vec<f32> = (0..1_000_000)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            f32::from_bits(0x3f80_0000 | (state >> 9))
        })
        .collect();
    let mut group = c.benchmark_group("robust_median_1m");
    group.sample_size(10);
    group.bench_function("sort", |bench| {
        bench.iter_batched(
            || samples.clone(),
            |mut values| {
                values.sort_by(f32::total_cmp);
                black_box(values[values.len() / 2]);
            },
            BatchSize::LargeInput,
        )
    });
    group.bench_function("select", |bench| {
        bench.iter_batched(
            || samples.clone(),
            |mut values| {
                black_box(median_f32(&mut values));
            },
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

criterion_group!(benches, median_bench);
criterion_main!(benches);
