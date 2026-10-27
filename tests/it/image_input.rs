//! Turning a decoded image into a model tensor never costs more memory than
//! the image itself, whatever its shape.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use image::{Rgb, RgbImage};
use openmod::config::{Filter, ImageInput, PixelRange, Resize};
use openmod::image_input::{shorter_side_crop, to_tensor};

fn spec(size: u32) -> ImageInput {
    ImageInput {
        size,
        resize: Resize::ShorterSideCrop,
        filter: Filter::Bicubic,
        pixel_range: PixelRange::Unit,
        mean: [0.5; 3],
        std: [0.5; 3],
    }
}

#[test]
fn the_square_cut_from_an_image_is_centred_and_never_larger_than_it() {
    // (case, width, height, the square: x, y, side)
    let cases: Vec<(&str, u32, u32, (u32, u32, u32))> = vec![
        ("a landscape photo", 400, 300, (50, 0, 300)),
        ("a portrait photo", 300, 400, (0, 50, 300)),
        ("a square", 384, 384, (0, 0, 384)),
        (
            "the widest strip the decoder takes",
            16_384,
            1,
            (8_191, 0, 1),
        ),
        (
            "the tallest strip the decoder takes",
            1,
            16_384,
            (0, 8_191, 1),
        ),
        ("an odd margin rounds down", 101, 100, (0, 0, 100)),
        ("an empty side is read as one pixel", 5, 0, (2, 0, 1)),
    ];
    let mut failures = Vec::new();
    for (case, w, h, want) in cases {
        let got = shorter_side_crop(w, h);
        if got != want {
            failures.push(format!("{case}: {got:?}, want {want:?}"));
        }
        let (x, y, side) = got;
        if x + side > w.max(1) || y + side > h.max(1) {
            failures.push(format!("{case}: the square {got:?} leaves the image"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_strip_of_an_image_becomes_a_tensor_of_the_model_size() {
    // (case, width, height)
    let cases = [
        ("a wide strip", 4_096, 1),
        ("a tall strip", 1, 4_096),
        ("a landscape photo", 640, 480),
    ];
    let mut failures = Vec::new();
    for (case, w, h) in cases {
        let image = RgbImage::from_pixel(w, h, Rgb([200, 100, 50]));
        let tensor = to_tensor(&image, &spec(64));
        if tensor.len() != 3 * 64 * 64 {
            failures.push(format!("{case}: {} values", tensor.len()));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
