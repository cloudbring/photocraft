//! HDR toning with representative short and long curves.
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use photocraft_algo::tone::{HdrToning, hdr_toning};

fn hdr_curve(c: &mut Criterion) {
    let width = 512;
    let pixels: Vec<_> = (0..width * width)
        .map(|index| {
            let value = (index % 1024) as f32 / 1023.0;
            [value, value, value, 1.0]
        })
        .collect();
    for count in [4, 256] {
        let curve = (0..count)
            .map(|index| {
                let value = index as f32 / (count - 1) as f32;
                (value, value * value)
            })
            .collect();
        let params = HdrToning { radius: 1.0, curve, ..Default::default() };
        c.bench_function(&format!("hdr_toning_512_{count}_knots"), |bench| {
            bench.iter_batched(|| pixels.clone(), |mut input| hdr_toning(&mut input, width, width, &params), BatchSize::LargeInput);
        });
    }
}

criterion_group! { name = benches; config = Criterion::default().sample_size(10); targets = hdr_curve }
criterion_main!(benches);
