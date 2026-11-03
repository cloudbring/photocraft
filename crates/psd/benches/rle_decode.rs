use criterion::{Criterion, criterion_group, criterion_main};
use photocraft_psd::compression::{Compression, PlaneLayout, decode_planes, encode_planes};
use photocraft_psd::header::Version;

fn rle_decode(criterion: &mut Criterion) {
    let layout = PlaneLayout { planes: 4, width: 8192, height: 8192, depth: 8, version: Version::Psb };
    let pixels: Vec<u8> = (0..layout.planes * layout.width * layout.height).map(|index| ((index / 8) % 251) as u8).collect();
    let encoded = encode_planes(Compression::Rle, &pixels, &layout).unwrap();
    drop(pixels);
    let mut group = criterion.benchmark_group("psd_rle_8k_4channel");
    group.sample_size(10);
    group.bench_function("decode", |bench| bench.iter(|| decode_planes(Compression::Rle, std::hint::black_box(&encoded), &layout).unwrap()));
    group.finish();
}

criterion_group!(benches, rle_decode);
criterion_main!(benches);
