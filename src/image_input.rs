//! Safe image decoding and model preprocessing.

use crate::config::{Filter, ImageInput, PixelRange, Resize};
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits, RgbImage};
use std::io::Cursor;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("the file is not a supported image (JPEG, PNG, WebP or GIF)")]
    Unsupported,
    #[error("the image is larger than {0} pixels")]
    TooLarge(u64),
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
    let side = 16_384;
    limits.max_image_width = Some(side);
    limits.max_image_height = Some(side);
    // RGBA8 worst case plus headroom for the decoder.
    limits.max_alloc = Some(
        max_pixels
            .saturating_mul(4)
            .saturating_add(64 * 1024 * 1024),
    );
    reader.limits(limits);

    let mut decoder = reader.into_decoder().map_err(map_err)?;
    let (w, h) = decoder.dimensions();
    if (w as u64) * (h as u64) > max_pixels {
        return Err(DecodeError::TooLarge(max_pixels));
    }
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(map_err)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img.to_rgb8())
}

fn map_err(e: image::ImageError) -> DecodeError {
    match e {
        image::ImageError::Unsupported(_) => DecodeError::Unsupported,
        image::ImageError::Limits(_) => DecodeError::TooLarge(0),
        other => DecodeError::Corrupt(other.to_string()),
    }
}

fn filter(f: Filter) -> FilterType {
    match f {
        Filter::Bilinear => FilterType::Triangle,
        Filter::Bicubic => FilterType::CatmullRom,
    }
}

/// Resizes and normalizes an image into a `[1, 3, size, size]` CHW tensor.
pub fn to_tensor(img: &RgbImage, spec: &ImageInput) -> Vec<f32> {
    let size = spec.size;
    let resized = match spec.resize {
        Resize::Stretch => image::imageops::resize(img, size, size, filter(spec.filter)),
        Resize::ShorterSideCrop => {
            let (w, h) = img.dimensions();
            let scale = size as f32 / w.min(h).max(1) as f32;
            let nw = ((w as f32 * scale).round() as u32).max(size);
            let nh = ((h as f32 * scale).round() as u32).max(size);
            let big = image::imageops::resize(img, nw, nh, filter(spec.filter));
            image::imageops::crop_imm(&big, (nw - size) / 2, (nh - size) / 2, size, size).to_image()
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
