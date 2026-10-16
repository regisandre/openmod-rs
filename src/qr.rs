//! QR codes in images: what they hold, so that a link in a QR code is
//! checked like a link in a text.
//!
//! Codes are searched on a grey copy of the image at most 2048 pixels on its
//! longer side, then on its negative when none was found, for codes printed
//! light on dark. The decoder (`rqrr`, a port of quirc) can panic on a
//! malformed image: the panic is caught and the image counts as holding no
//! code.

use image::RgbImage;
use image::imageops::FilterType;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The longer side codes are searched at.
pub const MAX_SIDE: u32 = 2048;

/// The most codes read from one image.
pub const MAX_CODES: usize = 8;

/// The smallest code is 21 modules wide.
const MIN_SIDE: u32 = 21;

fn grey(img: &RgbImage) -> (usize, usize, Vec<u8>) {
    let (w, h) = img.dimensions();
    let longer = w.max(h);
    let resized;
    let img = if longer > MAX_SIDE {
        let scale = MAX_SIDE as f32 / longer as f32;
        let nw = ((w as f32 * scale).round() as u32).max(1);
        let nh = ((h as f32 * scale).round() as u32).max(1);
        resized = image::imageops::resize(img, nw, nh, FilterType::Triangle);
        &resized
    } else {
        img
    };
    let luma = img
        .pixels()
        .map(|p| {
            let [r, g, b] = p.0;
            ((299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b)) / 1000) as u8
        })
        .collect();
    (img.width() as usize, img.height() as usize, luma)
}

fn decode(w: usize, h: usize, pixel: impl Fn(usize) -> u8) -> Vec<String> {
    let found = catch_unwind(AssertUnwindSafe(|| {
        let mut prepared =
            rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| pixel(y * w + x));
        let mut out: Vec<String> = Vec::new();
        for grid in prepared.detect_grids().into_iter().take(4 * MAX_CODES) {
            if let Ok((_, content)) = grid.decode()
                && !out.contains(&content)
            {
                out.push(content);
            }
            if out.len() == MAX_CODES {
                break;
            }
        }
        out
    }));
    found.unwrap_or_else(|_| {
        tracing::warn!("the QR decoder failed on an image: no code read from it");
        Vec::new()
    })
}

/// The content of every QR code readable in `img`, at most eight.
pub fn read(img: &RgbImage) -> Vec<String> {
    if img.width() < MIN_SIDE || img.height() < MIN_SIDE {
        return Vec::new();
    }
    let (w, h, luma) = grey(img);
    let found = decode(w, h, |i| luma[i]);
    if !found.is_empty() {
        return found;
    }
    decode(w, h, |i| 255 - luma[i])
}
