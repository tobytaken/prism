//! Screenshot comparison: draws a window with Slint's software renderer and compares it with a
//! reference PNG under a small color-distance threshold.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use i_slint_backend_testing::{TestingBackend, TestingBackendOptions};
use slint::{PhysicalSize, Rgba8Pixel, SharedPixelBuffer};

use super::{Backend, init_backend};

/// Set to `1` to regenerate the reference PNGs instead of comparing against them.
const UPDATE: &str = "PRISM_UPDATE_SCREENSHOTS";

/// How far a channel of a pixel may stray from the reference, out of 255, before it counts as
/// different. It absorbs antialiasing noise, not a changed color token.
const TOLERANCE: u8 = 8;

/// Sets up Slint's testing backend with the software renderer for this thread, so
/// `Window::take_snapshot` draws real pixels. Call it before building the window.
pub fn init_rendering_backend() {
    init_backend(Backend::Software, || {
        slint::platform::set_platform(Box::new(TestingBackend::new(TestingBackendOptions {
            mock_time: true,
            threading: false,
            renderer_name: Some("software".into()),
        })))
        .expect("no Slint platform is set up on this thread yet");
    });
}

/// Draws `window` at `size` (in pixels, at a scale factor of 1) and compares it with the reference
/// `tests/screenshots/<name>.png`. With `PRISM_UPDATE_SCREENSHOTS=1`, writes the reference
/// instead.
#[track_caller]
pub fn assert_screenshot(name: &str, window: &slint::Window, (width, height): (u32, u32)) {
    assert_eq!(
        window.scale_factor(),
        1.0,
        "screenshots draw at a scale factor of 1"
    );
    window.set_size(PhysicalSize::new(width, height));
    let actual = window.take_snapshot().expect("draw the window");
    let reference = reference_path(name);

    if std::env::var_os(UPDATE).is_some_and(|update| update == "1") {
        write_png(&reference, &actual);
        return;
    }

    let Some(expected) = read_png(&reference) else {
        panic!(
            "{name}: no reference at {}; run scripts/update-screenshots.sh to create it",
            reference.display()
        );
    };
    if let Some(difference) = difference(&expected, &actual) {
        let saved = actual_path(name);
        write_png(&saved, &actual);
        panic!(
            "{name}: {difference}\n  reference: {}\n  actual:    {}\nIf the change is intended, \
             run scripts/update-screenshots.sh",
            reference.display(),
            saved.display()
        );
    }
}

/// How `actual` differs from `expected`, or `None` when they match within the tolerance.
fn difference(
    expected: &SharedPixelBuffer<Rgba8Pixel>,
    actual: &SharedPixelBuffer<Rgba8Pixel>,
) -> Option<String> {
    let size = |image: &SharedPixelBuffer<Rgba8Pixel>| (image.width(), image.height());
    if size(expected) != size(actual) {
        return Some(format!(
            "the size changed from {:?} to {:?}",
            size(expected),
            size(actual)
        ));
    }
    let distance = |a: &Rgba8Pixel, b: &Rgba8Pixel| {
        [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)]
            .into_iter()
            .map(|(a, b)| a.abs_diff(b))
            .max()
            .unwrap_or_default()
    };
    let distances = expected
        .as_slice()
        .iter()
        .zip(actual.as_slice())
        .map(|(expected, actual)| distance(expected, actual));
    let (differing, furthest) = distances
        .filter(|distance| *distance > TOLERANCE)
        .fold((0, 0), |(count, furthest), distance| {
            (count + 1, furthest.max(distance))
        });
    (differing > 0).then(|| {
        format!("{differing} pixel(s) differ by more than {TOLERANCE}, at most by {furthest}")
    })
}

fn reference_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/screenshots")
        .join(format!("{name}.png"))
}

/// Where a mismatching screenshot is saved for a look.
fn actual_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("screenshots")
        .join(format!("{name}.png"))
}

fn read_png(path: &Path) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let file = File::open(path).ok()?;
    let mut reader = png::Decoder::new(BufReader::new(file))
        .read_info()
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut bytes = vec![
        0;
        reader
            .output_buffer_size()
            .expect("a PNG that fits in memory")
    ];
    let info = reader
        .next_frame(&mut bytes)
        .unwrap_or_else(|error| panic!("decode {}: {error}", path.display()));
    assert_eq!(
        (info.color_type, info.bit_depth),
        (png::ColorType::Rgba, png::BitDepth::Eight),
        "{} isn't 8-bit RGBA",
        path.display()
    );
    Some(SharedPixelBuffer::clone_from_slice(
        &bytes[..info.buffer_size()],
        info.width,
        info.height,
    ))
}

fn write_png(path: &Path, image: &SharedPixelBuffer<Rgba8Pixel>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = File::create(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let mut encoder = png::Encoder::new(BufWriter::new(file), image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(image.as_bytes()).unwrap();
    writer.finish().unwrap();
}
