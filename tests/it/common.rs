//! Fixtures shared by several themes: images with and without a QR code.

use image::{ImageFormat, Rgb, RgbImage};
use qrcode::{Color, QrCode};
use std::io::Cursor;

const DARK: Rgb<u8> = Rgb([20, 20, 30]);
const LIGHT: Rgb<u8> = Rgb([235, 230, 220]);

/// Draws a QR code of `payload` with `module` pixels per module, its quiet
/// zone included, at (`left`, `top`) of `img`.
pub fn draw(img: &mut RgbImage, payload: &str, module: u32, left: u32, top: u32, invert: bool) {
    let code = QrCode::new(payload.as_bytes()).unwrap();
    let width = code.width() as u32;
    let colors = code.to_colors();
    let (dark, light) = if invert { (LIGHT, DARK) } else { (DARK, LIGHT) };
    let side = (width + 8) * module;
    for y in 0..side {
        for x in 0..side {
            let (mx, my) = (x / module, y / module);
            let inside = (4..width + 4).contains(&mx) && (4..width + 4).contains(&my);
            let is_dark = inside && colors[((my - 4) * width + (mx - 4)) as usize] == Color::Dark;
            img.put_pixel(left + x, top + y, if is_dark { dark } else { light });
        }
    }
}

/// A photo-like background: a soft gradient, nothing that looks like a code.
pub fn background(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        Rgb([
            (120 + x * 60 / w) as u8,
            (90 + y * 80 / h) as u8,
            (140 + (x + y) * 40 / (w + h)) as u8,
        ])
    })
}

/// A photo-like background with, if given, a QR code of `payload` on it.
pub fn png(payload: Option<&str>) -> Vec<u8> {
    let mut img = background(900, 700);
    if let Some(p) = payload {
        draw(&mut img, p, 6, 400, 250, false);
    }
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}
