use photocraft_algo::{FilterParams, apply};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

#[test]
fn gaussian_impulse_variance() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
    for sigma in [0.5f32, 1.0, 2.0, 4.0, 8.0, 16.0, 64.0] {
        let half = (sigma * 8.0).ceil() as i32 + 8;
        let bounds = Rect::new(-half, -half, half + 1, half + 1);
        let mut source = Surface::new(format);
        source.fill_rect(Rect::new(0, 0, 1, 1), &[1.0]);
        let result = apply(&source, &FilterParams::GaussianBlur { radius: sigma }, bounds, bounds, None);
        let (mut mass, mut moment_x, mut moment_y) = (0.0f64, 0.0f64, 0.0f64);
        for vertical in bounds.y0..bounds.y1 {
            for horizontal in bounds.x0..bounds.x1 {
                let weight = f64::from(result.pixel(horizontal, vertical)[0]);
                mass += weight;
                moment_x += weight * f64::from(horizontal).powi(2);
                moment_y += weight * f64::from(vertical).powi(2);
            }
        }
        let variance = (moment_x + moment_y) / (2.0 * mass);
        let error = (variance / f64::from(sigma).powi(2) - 1.0) * 100.0;
        println!("sigma={sigma:>4}: mass={mass:.9}, variance={variance:.9}, error={error:+.6}%");
        assert!((mass - 1.0).abs() < 1e-4, "sigma={sigma}: mass={mass}");
        assert!(error.abs() <= 1.0, "sigma={sigma}: variance error={error}%");
        assert!((moment_x - moment_y).abs() < 1e-4 * f64::from(sigma).powi(2), "sigma={sigma}: anisotropic impulse");
    }
}
