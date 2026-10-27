//! The OCR pipeline around the two PP-OCR models: what the detector is shown,
//! how its probability map becomes text boxes, how the recognizer's output
//! becomes text. The models themselves are not needed.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::models::ocr::{Quad, ctc_decode, detection_size, lines, parse_dictionary, text_boxes};

#[test]
fn the_dictionary_is_read_from_the_file_published_with_the_recognizer() {
    let yml = "\
Global:
  model_name: latin_PP-OCRv5_mobile_rec
PostProcess:
  name: CTCLabelDecode
  character_dict:
  - '0'
  - A
  - é
  - ''''
  - '\"'
  - \"\\\\\"
  - '#'
";
    // (case, index, character)
    let cases: Vec<(&str, usize, &str)> = vec![
        ("a quoted digit", 0, "0"),
        ("a plain letter", 1, "A"),
        ("a letter outside ASCII", 2, "é"),
        ("a single quote, doubled inside quotes", 3, "'"),
        ("a double quote inside single quotes", 4, "\""),
        ("an escaped backslash inside double quotes", 5, "\\"),
        ("a character YAML would read as a comment", 6, "#"),
    ];
    let dict = parse_dictionary(yml).unwrap();
    assert_eq!(dict.len(), 7, "every entry, nothing after the list");
    for (case, index, expected) in cases {
        assert_eq!(dict[index], expected, "{case}");
    }
    assert!(
        parse_dictionary("Global:\n  model_name: x\n").is_err(),
        "a file without a dictionary is refused"
    );
}

#[test]
fn the_detector_sees_the_image_scaled_down_to_multiples_of_32() {
    // (case, width, height, longest side, expected size)
    let cases: Vec<(&str, u32, u32, u32, (u32, u32))> = vec![
        ("a poster, scaled down", 1080, 1350, 960, (768, 960)),
        ("a landscape photo", 4000, 2000, 960, (960, 480)),
        ("a small image is not enlarged", 640, 480, 960, (640, 480)),
        ("sides rounded to 32", 100, 50, 960, (96, 64)),
        ("never below 32", 10, 10, 960, (32, 32)),
    ];
    for (case, w, h, max_side, expected) in cases {
        assert_eq!(detection_size(w, h, max_side), expected, "{case}");
    }
}

/// A `w x h` map at 0.01 with `value` over the given pixel rectangles.
fn map(w: usize, h: usize, regions: &[(usize, usize, usize, usize, f32)]) -> Vec<f32> {
    let mut m = vec![0.01f32; w * h];
    for &(x0, y0, x1, y1, value) in regions {
        for y in y0..y1 {
            for x in x0..x1 {
                m[y * w + x] = value;
            }
        }
    }
    m
}

fn side(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

#[test]
fn each_text_region_becomes_one_box_grown_like_db_and_specks_are_dropped() {
    let (w, h) = (240, 160);
    // A slanted line: a band 9 px thick along y = 0.5 x + 60.
    let mut slanted = map(w, h, &[]);
    for x in 40..180usize {
        let mid = 0.5 * x as f32 + 60.0;
        for y in 0..h {
            if (y as f32 - mid).abs() <= 4.0 {
                slanted[y * w + x] = 0.9;
            }
        }
    }
    // A line through pixel centers 20..=119 and 30..=43 is 99 x 13: DB grows
    // it by area * 1.5 / perimeter = 1287 * 1.5 / 224 = 8.62 on each side.
    let grown = 1287.0 * 1.5 / 224.0;
    // (case, map, expected boxes, (top-left, long side, short side, angle in degrees) of the first)
    let cases: Vec<(&str, Vec<f32>, usize, Option<((f32, f32), f32, f32, f32)>)> = vec![
        (
            "a line of text",
            map(w, h, &[(20, 30, 120, 44, 0.9)]),
            1,
            Some((
                (20.0 - grown, 30.0 - grown),
                99.0 + 2.0 * grown,
                13.0 + 2.0 * grown,
                0.0,
            )),
        ),
        (
            "two lines",
            map(w, h, &[(20, 30, 120, 44, 0.9), (20, 90, 200, 110, 0.8)]),
            2,
            None,
        ),
        ("a speck", map(w, h, &[(50, 50, 52, 52, 0.95)]), 0, None),
        (
            "a region the detector is unsure about",
            map(w, h, &[(20, 30, 120, 44, 0.4)]),
            0,
            None,
        ),
        ("nothing", map(w, h, &[]), 0, None),
        (
            "a slanted line keeps its angle",
            slanted,
            1,
            Some(((f32::NAN, f32::NAN), f32::NAN, f32::NAN, 26.57)),
        ),
    ];
    for (case, m, count, first) in cases {
        let boxes: Vec<Quad> = text_boxes(&m, w, h);
        assert_eq!(boxes.len(), count, "{case}: boxes");
        let Some((top_left, long, short, angle)) = first else {
            continue;
        };
        let [tl, tr, _, bl] = boxes[0];
        let close = |got: f32, want: f32, tol: f32| want.is_nan() || (got - want).abs() <= tol;
        assert!(
            close(tl.0, top_left.0, 1.0) && close(tl.1, top_left.1, 1.0),
            "{case}: top-left {tl:?}"
        );
        let (top, left) = (side(tl, tr), side(tl, bl));
        assert!(close(top, long, 1.5), "{case}: long side {top}");
        assert!(close(left, short, 1.5), "{case}: short side {left}");
        let got = (tr.1 - tl.1).atan2(tr.0 - tl.0).to_degrees();
        assert!(close(got, angle, 2.0), "{case}: angle {got}");
    }
}

#[test]
fn the_recognizer_output_is_read_greedily_collapsing_repeats_and_blanks() {
    let dict: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    // Classes: 0 blank, 1..=3 the dictionary, 4 a space.
    let steps = |path: &[(usize, f32)]| -> Vec<f32> {
        let mut out = vec![0.0f32; path.len() * 5];
        for (t, &(class, p)) in path.iter().enumerate() {
            out[t * 5 + class] = p;
            out[t * 5 + if class == 0 { 1 } else { 0 }] = 1.0 - p;
        }
        out
    };
    // (case, path of (class, probability), text, confidence)
    let cases: Vec<(&str, Vec<(usize, f32)>, &str, f32)> = vec![
        (
            "a repeat is one letter, a blank separates two",
            vec![(1, 0.9), (1, 0.7), (0, 0.9), (1, 0.8), (2, 0.6)],
            "aab",
            (0.9 + 0.8 + 0.6) / 3.0,
        ),
        (
            "the last class is a space",
            vec![(3, 1.0), (4, 1.0), (3, 1.0)],
            "c c",
            1.0,
        ),
        ("only blanks", vec![(0, 0.9), (0, 0.9)], "", 0.0),
    ];
    for (case, path, text, confidence) in cases {
        let (got, score) = ctc_decode(&steps(&path), 5, &dict);
        assert_eq!(got, text, "{case}");
        assert!(
            (score - confidence).abs() < 1e-4,
            "{case}: confidence {score}"
        );
    }
}

#[test]
fn words_boxed_apart_are_joined_back_into_their_lines() {
    let word = |x: f32, y: f32, h: f32, text: &str| -> (Quad, String) {
        let w = 20.0 * text.len() as f32;
        (
            [(x, y), (x + w, y), (x + w, y + h), (x, y + h)],
            text.to_string(),
        )
    };
    // (case, boxes in reading order, lines)
    let cases: Vec<(&str, Vec<(Quad, String)>, Vec<&str>)> = vec![
        (
            "two words of a title, then the next line",
            vec![
                word(80.0, 100.0, 60.0, "TECHNO"),
                word(300.0, 102.0, 60.0, "NIGHT"),
                word(80.0, 200.0, 30.0, "Saturday"),
            ],
            vec!["TECHNO NIGHT", "Saturday"],
        ),
        (
            "a slanted line drifts down word after word",
            vec![
                word(80.0, 100.0, 40.0, "bet365.com"),
                word(300.0, 115.0, 40.0, "official"),
                word(480.0, 130.0, 40.0, "partner"),
            ],
            vec!["bet365.com official partner"],
        ),
        (
            "a small caption under a big title is its own line",
            vec![
                word(80.0, 100.0, 120.0, "SUNBREAK"),
                word(80.0, 205.0, 20.0, "tickets"),
            ],
            vec!["SUNBREAK", "tickets"],
        ),
        ("nothing", vec![], vec![]),
    ];
    for (case, boxes, expected) in cases {
        assert_eq!(lines(&boxes), expected, "{case}");
    }
}
