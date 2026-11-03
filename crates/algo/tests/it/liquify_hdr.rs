use photocraft_algo::liquify::{LiquifyField, LiquifyStroke, LiquifyTool, apply_liquify};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

#[test]
fn float_warp_preserves_hdr_and_signed_colors() {
    let bounds = Rect::new(0, 0, 64, 64);
    let mut stroke = LiquifyStroke::new(LiquifyTool::ForwardWarp, 48.0);
    stroke.points = vec![vec![32.0, 32.0], vec![36.0, 32.0]];
    let field = LiquifyField::from_strokes(bounds, 4.0, &[stroke]);
    assert!(!field.is_identity());
    for alpha in [false, true] {
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, alpha);
        let pixel: Vec<f32> = if alpha { vec![4.0, -0.2, 0.5, 0.5] } else { vec![4.0, -0.2, 0.5] };
        let mut source = Surface::new(format);
        source.write_region(bounds, &pixel.repeat(64 * 64));
        let result = apply_liquify(&source, &field);
        let actual = result.pixel(32, 32);
        for (actual, expected) in actual.iter().zip(&pixel) {
            assert!((actual - expected).abs() < 1e-5, "alpha={alpha}: {actual} != {expected}");
        }
    }
}

#[test]
fn transparent_edges_keep_signed_premultiplied_color() {
    let bounds = Rect::new(0, 0, 64, 64);
    let mut stroke = LiquifyStroke::new(LiquifyTool::ForwardWarp, 48.0);
    stroke.points = vec![vec![32.0, 32.0], vec![36.0, 32.0]];
    let field = LiquifyField::from_strokes(bounds, 4.0, &[stroke]);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    let mut pixels = Vec::new();
    for _ in 0..64 {
        for x in 0..64 {
            pixels.extend_from_slice(if x < 32 { &[4.0, -0.2, 0.5, 0.5] } else { &[7.0, -2.0, 1.0, 0.0] });
        }
    }
    source.write_region(bounds, &pixels);
    let result = apply_liquify(&source, &field);
    let mut partial = 0;
    for x in 20..44 {
        let pixel = result.pixel(x, 32);
        if pixel[3] > 0.001 && pixel[3] < 0.499 {
            partial += 1;
            for (actual, expected) in pixel[..3].iter().zip([4.0, -0.2, 0.5]) {
                assert!((actual - expected).abs() < 1e-4, "{pixel:?}");
            }
        } else if pixel[3] == 0.0 {
            assert_eq!(&pixel[..3], &[0.0, 0.0, 0.0]);
        }
    }
    assert!(partial > 0);
}
