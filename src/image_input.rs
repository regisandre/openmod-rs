//! Safe image decoding and model preprocessing.

use crate::config::{Filter, ImageInput, PixelRange, Resize};
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits, RgbImage};
use std::io::Cursor;

/// The longest side the decoder takes, whatever `max_image_pixels` allows.
pub const MAX_SIDE: u32 = 16_384;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("the file is not a supported image (JPEG, PNG, WebP or GIF)")]
    Unsupported,
    #[error("the image is larger than {0} pixels")]
    TooLarge(u64),
    /// The decoder's own limits tripped: a side over `MAX_SIDE`, or more
    /// memory than `max_image_pixels` needs.
    #[error(
        "the image is too large to decode: at most {side} pixels a side and {0} pixels in all",
        side = MAX_SIDE
    )]
    OverLimits(u64),
    #[error("the image could not be decoded: {0}")]
    Corrupt(String),
}

/// Decodes untrusted bytes with hard limits on dimensions and memory, and
/// applies the EXIF orientation so a rotated photo is seen upright.
pub fn decode(bytes: &[u8], max_pixels: u64) -> Result<RgbImage, DecodeError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| DecodeError::Corrupt(e.to_string()))?;
    if reader.format().is_none() {
        return Err(DecodeError::Unsupported);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    // RGBA8 worst case plus headroom for the decoder.
    limits.max_alloc = Some(
        max_pixels
            .saturating_mul(4)
            .saturating_add(64 * 1024 * 1024),
    );
    reader.limits(limits);

    let refused = |e| map_err(e, max_pixels);
    let mut decoder = reader.into_decoder().map_err(refused)?;
    let (w, h) = decoder.dimensions();
    if (w as u64) * (h as u64) > max_pixels {
        return Err(DecodeError::TooLarge(max_pixels));
    }
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(refused)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img.to_rgb8())
}

fn map_err(e: image::ImageError, max_pixels: u64) -> DecodeError {
    match e {
        image::ImageError::Unsupported(_) => DecodeError::Unsupported,
        image::ImageError::Limits(_) => DecodeError::OverLimits(max_pixels),
        other => DecodeError::Corrupt(other.to_string()),
    }
}

fn filter(f: Filter) -> FilterType {
    match f {
        Filter::Bilinear => FilterType::Triangle,
        Filter::Bicubic => FilterType::CatmullRom,
    }
}

/// The centred square a `ShorterSideCrop` keeps of a `w` x `h` image: its
/// corner and its side. It never leaves the image, and an empty side counts
/// as one pixel.
pub fn shorter_side_crop(w: u32, h: u32) -> (u32, u32, u32) {
    let side = w.min(h).max(1);
    (w.saturating_sub(side) / 2, h.saturating_sub(side) / 2, side)
}

/// Resizes and normalizes an image into a `[1, 3, size, size]` CHW tensor.
pub fn to_tensor(img: &RgbImage, spec: &ImageInput) -> Vec<f32> {
    let size = spec.size;
    let resized = match spec.resize {
        Resize::Stretch => image::imageops::resize(img, size, size, filter(spec.filter)),
        Resize::ShorterSideCrop => {
            // Cut the centred square first, then resize it: scaling the whole
            // image up to the model size would turn a 16384 x 1 strip into a
            // multi-gigabyte buffer.
            let (w, h) = img.dimensions();
            let (x, y, side) = shorter_side_crop(w, h);
            let square = image::imageops::crop_imm(img, x, y, side, side);
            image::imageops::resize(&*square, size, size, filter(spec.filter))
        }
    };

    let plane = (size * size) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (i, px) in resized.pixels().enumerate() {
        for c in 0..3 {
            let v = px.0[c] as f32;
            out[c * plane + i] = match spec.pixel_range {
                PixelRange::Byte => v,
                PixelRange::Unit => (v / 255.0 - spec.mean[c]) / spec.std[c],
            };
        }
    }
    out
}
