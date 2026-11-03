use super::*;
use craft_blur::{Cancel, Gaussian, Quality, Scratch, Shape};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_raster::Surface;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn recursive_matches_shared_core_across_depths_models_and_alpha() {
    let extent = Rect::new(-13, 7, 72, 38);
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for mode in [ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Multichannel] {
            for alpha in [false, true] {
                let format = PixelFormat::new(mode, sample, alpha);
                let channels = format.channels();
                let mut source = Surface::new(format);
                let values: Vec<f32> =
                    (0..extent.width() as usize * extent.height() as usize * channels)
                        .map(|index| {
                            if alpha && index % channels == channels - 1 { [0.0, 0.25, 1.0][(index / channels) % 3] } else { (index * 73 % 113) as f32 / 113.0 }
                        })
                        .collect();
                source.write_region(extent, &values);
                for sigma in [8.0, 16.0, 64.0] {
                    let filter = Gaussian::new(sigma, Quality::Recursive).unwrap();
                    let mut expected = source.read_region(extent);
                    premultiply(&mut expected, channels, alpha);
                    for channel in 0..channels {
                        let mut plane: Vec<f32> = expected.chunks_exact(channels).map(|pixel| pixel[channel]).collect();
                        let shape = Shape::new(extent.width() as usize, extent.height() as usize, 1).unwrap();
                        filter.apply(&mut plane, shape, craft_blur::Edge::Clamp, &mut Scratch::default(), Cancel::Never).unwrap();
                        for (pixel, value) in expected.chunks_exact_mut(channels).zip(plane) {
                            pixel[channel] = value;
                        }
                    }
                    unpremultiply(&mut expected, channels, alpha);
                    let mut quantized = Surface::new(format);
                    quantized.write_region(extent, &expected);
                    let expected = quantized.read_region(extent);
                    let params = FilterParams::GaussianBlur { radius: sigma };
                    for tile in [1, 37, 256] {
                        let output = crate::apply_tiled_with(&source, &params, extent, extent, None, tile, Some(extent), &Interrupt::NONE).unwrap();
                        let actual = output.read_region(extent);
                        let delta = actual.iter().zip(&expected).map(|(actual, expected)| (actual - expected).abs()).fold(0.0f32, f32::max);
                        assert!(delta < 1e-6, "{format:?}, sigma={sigma}, tile={tile}: delta={delta}");
                    }
                }
            }
        }
    }
}

#[test]
fn recursive_reads_full_extent_for_roi_and_cancels_in_core() {
    let extent = Rect::new(-40, -20, 90, 70);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    source.fill_rect(extent, &[0.8, 0.2, 0.1, 1.0]);
    source.fill_rect(Rect::new(10, 5, 40, 40), &[5.0, -2.0, 0.5, 0.0]);
    let original = source.read_region(extent);
    let params = FilterParams::GaussianBlur { radius: 16.0 };
    let whole = crate::apply_in_with(&source, &params, extent, extent, None, extent, &Interrupt::NONE).unwrap();
    for area in [Rect::new(-40, -20, -25, 0), Rect::new(0, 5, 67, 30), Rect::new(83, 63, 90, 70)] {
        let cropped = crate::apply_in_with(&source, &params, area, extent, None, extent, &Interrupt::NONE).unwrap();
        assert_eq!(whole.read_region(area), cropped.read_region(area));
        for pixel in cropped.read_region(area).as_chunks::<4>().0 {
            assert!((pixel[0] - 0.8).abs() < 1e-6 && (pixel[1] - 0.2).abs() < 1e-6, "hidden colors must not bleed");
        }
    }
    let checks = AtomicUsize::new(0);
    let count = || {
        checks.fetch_add(1, Ordering::SeqCst);
        false
    };
    let ctl = Interrupt::cancel_only(&count);
    assert!(crate::apply_in_with(&source, &params, extent, extent, None, extent, &ctl).is_some());
    let total = checks.load(Ordering::SeqCst);
    assert!(total > 8);
    for stop in [0, 1, 8, total / 2, total - 1] {
        let seen = AtomicUsize::new(0);
        let cancel = || seen.fetch_add(1, Ordering::SeqCst) >= stop;
        let ctl = Interrupt::cancel_only(&cancel);
        assert!(crate::apply_in_with(&source, &params, extent, extent, None, extent, &ctl).is_none(), "check={stop}");
        assert_eq!(source.read_region(extent), original);
    }
}

#[test]
fn recursive_repeats_source_edges_before_blurring_outside_extent() {
    let extent = Rect::new(-10, -5, 21, 18);
    let area = extent.inflate(12);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    source.fill_rect(extent, &[0.2, 0.6, 0.8, 0.5]);
    source.fill_rect(Rect::new(5, 3, 21, 18), &[0.9, 0.1, 0.4, 1.0]);
    let mut expected = Image::read_clamped(&source, area, extent).into_data();
    premultiply(&mut expected, 4, true);
    Gaussian::new(8.0, Quality::Recursive)
        .unwrap()
        .apply(
            &mut expected,
            Shape::new(area.width() as usize, area.height() as usize, 4).unwrap(),
            craft_blur::Edge::Clamp,
            &mut Scratch::default(),
            Cancel::Never,
        )
        .unwrap();
    unpremultiply(&mut expected, 4, true);
    let params = FilterParams::GaussianBlur { radius: 8.0 };
    let actual = crate::apply_tiled_with(&source, &params, area, extent, None, 37, Some(extent), &Interrupt::NONE).unwrap().read_region(area);
    let delta = actual.iter().zip(expected).map(|(actual, expected)| (actual - expected).abs()).fold(0.0f32, f32::max);
    assert!(delta < 1e-6, "outside-extent delta={delta}");
    let content = Rect::new(-4, 2, 9, 11);
    assert_eq!(crate::output_area(&params, content, extent, None), content.inflate(25));
    assert_eq!(crate::output_area(&params, Rect::EMPTY, extent, None), Rect::EMPTY);
}

#[test]
fn migration_fixture_deltas_from_main() {
    let extent = Rect::new(0, 0, 96, 64);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    source.fill_rect(extent, &[0.0, 0.0, 0.0, 1.0]);
    source.fill_rect(Rect::new(40, 0, 96, 64), &[1.0, 0.3, 0.6, 1.0]);
    source.fill_rect(Rect::new(10, 20, 70, 45), &[0.1, 0.8, 1.0, 0.25]);
    source.fill_rect(Rect::new(20, 25, 30, 35), &[1.0, 0.0, 1.0, 0.0]);
    for sigma in [0.3f32, 0.8, 1.0, 2.0, 3.0, 4.0, 8.0, 16.0, 64.0] {
        let reach = (sigma * 3.0).ceil() as i32 + 1;
        let image = Image::read_clamped(&source, extent.inflate(reach), extent);
        let mut expected = if sigma > 4.0 {
            box_passes(&image, extent, true, &boxes_for_gauss(sigma, 3))
        } else {
            let kernel = legacy_kernel(sigma);
            conv_sep(&image, extent, &kernel, &kernel, true)
        };
        let mut actual = crate::apply_in(&source, &FilterParams::GaussianBlur { radius: sigma }, extent, extent, None, extent).read_region(extent);
        let delta = actual.iter().zip(&expected).map(|(actual, expected)| (actual - expected).abs()).fold(0.0f32, f32::max);
        premultiply(&mut actual, 4, true);
        premultiply(&mut expected, 4, true);
        let weighted = actual.iter().zip(&expected).map(|(actual, expected)| (actual - expected).abs()).fold(0.0f32, f32::max);
        println!("migration fixture RGBA32F sigma={sigma}: max_delta={delta:.9}, premultiplied={weighted:.9}");
        assert!(weighted < 0.15, "sigma={sigma}: unexpectedly large weighted delta={weighted}");
    }
}

fn legacy_kernel(sigma: f32) -> Vec<f32> {
    if sigma >= 1.0 {
        return gaussian_kernel(sigma);
    }
    let sample = |width: f32| {
        let weights: Vec<f32> = (-3i32..=3).map(|offset| (-(offset * offset) as f32 / (2.0 * width * width)).exp()).collect();
        let mass: f32 = weights.iter().sum();
        weights.into_iter().map(|weight| weight / mass).collect::<Vec<f32>>()
    };
    let (mut low, mut high) = (sigma, sigma + 1.0);
    for _ in 0..40 {
        let middle = (low + high) * 0.5;
        let variance: f32 = sample(middle).iter().enumerate().map(|(index, weight)| weight * (index as f32 - 3.0).powi(2)).sum();
        if variance < sigma * sigma {
            low = middle;
        } else {
            high = middle;
        }
    }
    sample((low + high) * 0.5)
}
