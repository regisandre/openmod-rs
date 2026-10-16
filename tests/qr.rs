//! QR codes in images: what they hold, read like a link in a text.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use image::{ImageFormat, Rgb, RgbImage};
use openmod::config::Config;
use openmod::decision::Action;
use openmod::engine::Engine;
use openmod::qr;
use qrcode::{Color, QrCode};
use std::io::Cursor;

const DARK: Rgb<u8> = Rgb([20, 20, 30]);
const LIGHT: Rgb<u8> = Rgb([235, 230, 220]);

/// Draws a QR code of `payload` with `module` pixels per module, its quiet
/// zone included, at (`left`, `top`) of `img`.
fn draw(img: &mut RgbImage, payload: &str, module: u32, left: u32, top: u32, invert: bool) {
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
fn background(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        Rgb([
            (120 + x * 60 / w) as u8,
            (90 + y * 80 / h) as u8,
            (140 + (x + y) * 40 / (w + h)) as u8,
        ])
    })
}

#[test]
fn qr_codes_are_read_wherever_they_sit_and_nothing_else_is() {
    let url = "https://casino-x.com/promo?ref=42";
    let mut poster = background(1200, 900);
    draw(&mut poster, url, 6, 820, 520, false);

    let mut inverted = background(1200, 900);
    draw(&mut inverted, url, 6, 100, 100, true);

    let mut two = background(1400, 700);
    draw(&mut two, "https://first.example/a", 6, 80, 120, false);
    draw(&mut two, "WIFI:S:club;T:WPA;P:secret;;", 6, 760, 120, false);

    let mut large = background(4000, 3000);
    draw(&mut large, url, 14, 2600, 1800, false);

    // (case, image, payloads found)
    let cases: Vec<(&str, RgbImage, Vec<&str>)> = vec![
        ("a code in the corner of a poster", poster, vec![url]),
        ("a code printed light on dark", inverted, vec![url]),
        (
            "two codes, one of them not a link",
            two,
            vec!["WIFI:S:club;T:WPA;P:secret;;", "https://first.example/a"],
        ),
        ("a code in a 12 MP photo, read at 2048 px", large, vec![url]),
        ("a photo without a code", background(1200, 900), vec![]),
        ("an image smaller than any code", background(12, 12), vec![]),
    ];
    for (case, img, expected) in cases {
        let mut got = qr::read(&img);
        got.sort();
        let mut expected: Vec<String> = expected.into_iter().map(String::from).collect();
        expected.sort();
        assert_eq!(got, expected, "{case}");
    }
}

/// A server with the QR reader and a one-site adult list, and the workflow
/// `rules`, in a scratch directory.
fn engine(name: &str, list_enabled: bool, rules: &str) -> Engine {
    let dir = std::env::temp_dir().join(format!("openmod-qr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lists")).unwrap();
    std::fs::write(dir.join("lists/porn.txt"), "0.0.0.0 adult-site.com\n").unwrap();
    std::fs::write(dir.join("lists/psl.dat"), "com\n").unwrap();
    let config = format!(
        r#"
[models.qr]
kind = "qr_reader"

[models.adult_sites]
kind = "domain_list"
enabled = {list_enabled}
paths = ["lists/porn.txt"]
public_suffixes = "lists/psl.dat"
category = "link_adult"

[workflows.default.image]
{rules}
"#
    );
    std::fs::write(dir.join("openmod.toml"), config).unwrap();
    Engine::load(Config::load(&dir.join("openmod.toml")).unwrap(), None)
}

fn png(payload: Option<&str>) -> Vec<u8> {
    let mut img = background(900, 700);
    if let Some(p) = payload {
        draw(&mut img, p, 6, 400, 250, false);
    }
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

#[test]
fn a_link_in_a_qr_code_is_checked_like_a_link_in_a_text() {
    let rules =
        "qr_link_adult = { reject = 1.0 }\nqr_link = { review = 1.0 }\nqr_code = { review = 1.0 }";
    let server = engine("link", true, rules);
    // (case, QR code payload, action, reasons)
    let cases: Vec<(&str, Option<&str>, Action, Vec<&str>)> = vec![
        (
            "a code to an adult site",
            Some("https://www.adult-site.com/promo"),
            Action::Reject,
            vec!["qr_link_adult"],
        ),
        (
            "a code to another site",
            Some("https://night.events/e/12"),
            Action::Review,
            vec!["qr_code", "qr_link"],
        ),
        (
            "a code without a link",
            Some("WIFI:S:club;T:WPA;P:secret;;"),
            Action::Review,
            vec!["qr_code"],
        ),
        ("no code", None, Action::Accept, vec![]),
    ];
    for (case, payload, action, reasons) in cases {
        let report = server.check_image(&png(payload), "default").unwrap();
        assert_eq!(report.summary.action, action, "{case}");
        let mut got: Vec<&str> = match action {
            Action::Reject => &report.summary.reject_reason,
            _ => &report.summary.review_reason,
        }
        .iter()
        .map(|r| r.id.as_str())
        .collect();
        got.sort();
        assert_eq!(got, reasons, "{case}: reasons");
    }
    let off = engine("off", false, rules);
    let report = off
        .check_image(&png(Some("https://adult-site.com")), "default")
        .unwrap();
    assert_eq!(
        report.unchecked,
        vec!["qr_link_adult"],
        "a rule whose list is off is reported unchecked"
    );
    assert_eq!(
        report.summary.action,
        Action::Review,
        "the other rules still run"
    );
}
