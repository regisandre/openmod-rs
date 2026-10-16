//! PP-OCRv5: the text lines of an image, found by a detection model (DB) and
//! read by a recognition model (CTC), both ONNX, with PaddleOCR's own
//! processing around them:
//! - the detector sees the image at most `max_side` pixels on its longer
//!   side, each side rounded to a multiple of 32, in BGR with ImageNet's
//!   mean and deviation;
//! - its probability map is cut at 0.3; each connected region becomes the
//!   smallest rectangle around it, kept when the map averages 0.6 or more
//!   inside, then grown on every side by area * 1.5 / perimeter (DB's
//!   "unclip");
//! - each rectangle is cut out of the full-size image, turned upright when
//!   it is much taller than wide, scaled to 48 pixels high and read: the
//!   most likely character at each step, repeats collapsed and blanks
//!   dropped, is the text, and the mean probability of its characters is
//!   its confidence.

use crate::models::onnx::{Runtime, open_sessions};
use crate::pool::Pool;
use anyhow::{Result, anyhow, bail};
use image::imageops::FilterType;
use image::{Rgb, RgbImage};
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::Path;

const BIN_THRESHOLD: f32 = 0.3;
const BOX_THRESHOLD: f32 = 0.6;
const UNCLIP_RATIO: f32 = 1.5;
const MIN_SIDE: f32 = 3.0;
/// The most regions of the probability map turned into boxes.
const MAX_REGIONS: usize = 1000;
/// The most lines read from one image.
const MAX_LINES: usize = 200;
const REC_HEIGHT: u32 = 48;
const REC_MIN_WIDTH: u32 = 320;
const REC_MAX_WIDTH: u32 = 3200;
const REC_BATCH: usize = 6;
/// Applied to the B, G and R planes in this order, as PaddleOCR does.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// A text box: top-left, top-right, bottom-right, bottom-left.
pub type Quad = [(f32, f32); 4];

/// The characters the recognizer reads, from the `character_dict` list of
/// the `inference.yml` PaddleOCR publishes with it. The recognizer has two
/// more classes: a blank first, a space last.
pub fn parse_dictionary(yml: &str) -> Result<Vec<String>> {
    let mut lines = yml.lines().skip_while(|l| l.trim() != "character_dict:");
    if lines.next().is_none() {
        bail!("no `character_dict` list in the dictionary file");
    }
    let mut out = Vec::new();
    for line in lines {
        let Some(item) = line.trim_start().strip_prefix("- ") else {
            break;
        };
        out.push(yaml_scalar(item.trim_end()));
    }
    if out.is_empty() {
        bail!("the `character_dict` list of the dictionary file is empty");
    }
    Ok(out)
}

/// A YAML scalar as the dictionary writes them: plain, `'single'` (with
/// `''` for a quote) or `"double"` (with backslash escapes).
fn yaml_scalar(s: &str) -> String {
    let quoted = |q: char| s.len() >= 2 && s.starts_with(q) && s.ends_with(q);
    if quoted('\'') {
        return s[1..s.len() - 1].replace("''", "'");
    }
    if quoted('"') {
        let mut out = String::new();
        let mut chars = s[1..s.len() - 1].chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        }
        return out;
    }
    s.to_string()
}

/// The size the detector sees: at most `max_side` on the longer side, never
/// enlarged, each side rounded to a multiple of 32.
pub fn detection_size(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    let longer = w.max(h).max(1);
    let ratio = if longer > max_side {
        max_side as f32 / longer as f32
    } else {
        1.0
    };
    let side = |v: u32| {
        let scaled = (v as f32 * ratio) as u32;
        ((scaled as f32 / 32.0).round() as u32 * 32).max(32)
    };
    (side(w), side(h))
}

fn side(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn cross(o: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// Andrew's monotone chain.
fn convex_hull(mut points: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let mut hull: Vec<(f32, f32)> = Vec::with_capacity(points.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &(f32, f32)>> = if pass == 0 {
            Box::new(points.iter())
        } else {
            Box::new(points.iter().rev())
        };
        for &p in iter {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

/// A rectangle: its center, the unit vector along its first side, and its
/// two side lengths.
#[derive(Debug, Clone, Copy)]
struct Rect {
    center: (f32, f32),
    axis: (f32, f32),
    w: f32,
    h: f32,
}

impl Rect {
    /// The smallest rectangle around `hull` (rotating calipers).
    fn around(hull: &[(f32, f32)]) -> Rect {
        let mut best: Option<(f32, Rect)> = None;
        let n = hull.len();
        for i in 0..n.max(1) {
            let (a, b) = (hull[i], hull[(i + 1) % n]);
            let len = side(a, b);
            let axis = if len > 0.0 {
                ((b.0 - a.0) / len, (b.1 - a.1) / len)
            } else {
                (1.0, 0.0)
            };
            let normal = (-axis.1, axis.0);
            let (mut u0, mut u1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            for p in hull {
                let u = p.0 * axis.0 + p.1 * axis.1;
                let v = p.0 * normal.0 + p.1 * normal.1;
                (u0, u1, v0, v1) = (u0.min(u), u1.max(u), v0.min(v), v1.max(v));
            }
            let (um, vm) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
            let rect = Rect {
                center: (um * axis.0 + vm * normal.0, um * axis.1 + vm * normal.1),
                axis,
                w: u1 - u0,
                h: v1 - v0,
            };
            let area = rect.w * rect.h;
            if best.is_none_or(|(a, _)| area < a) {
                best = Some((area, rect));
            }
        }
        best.map(|(_, r)| r).expect("a hull has a point")
    }

    fn grown(self, by: f32) -> Rect {
        Rect {
            w: self.w + 2.0 * by,
            h: self.h + 2.0 * by,
            ..self
        }
    }

    /// The corners, ordered as PaddleOCR orders them: of the two leftmost,
    /// the higher is the top-left; of the two rightmost, the top-right.
    fn corners(&self) -> Quad {
        let (c, u) = (self.center, self.axis);
        let v = (-u.1, u.0);
        let (hw, hh) = (self.w / 2.0, self.h / 2.0);
        let mut pts = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
            .map(|(a, b)| (c.0 + a * u.0 + b * v.0, c.1 + a * u.1 + b * v.1));
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (left, right) = pts.split_at_mut(2);
        left.sort_by(|a, b| a.1.total_cmp(&b.1));
        right.sort_by(|a, b| a.1.total_cmp(&b.1));
        [left[0], right[0], right[1], left[1]]
    }
}

/// The mean of the map over the pixels inside `quad`.
fn mean_inside(prob: &[f32], w: usize, h: usize, quad: &Quad) -> f32 {
    let xs = quad.map(|p| p.0);
    let ys = quad.map(|p| p.1);
    let clamp = |v: f32, max: usize| (v.max(0.0) as usize).min(max - 1);
    let (x0, x1) = (
        clamp(xs.iter().copied().fold(f32::MAX, f32::min).floor(), w),
        clamp(xs.iter().copied().fold(f32::MIN, f32::max).ceil(), w),
    );
    let (y0, y1) = (
        clamp(ys.iter().copied().fold(f32::MAX, f32::min).floor(), h),
        clamp(ys.iter().copied().fold(f32::MIN, f32::max).ceil(), h),
    );
    let (mut sum, mut n) = (0.0f32, 0usize);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let p = (x as f32, y as f32);
            let inside = (0..4).all(|i| {
                let (a, b) = (quad[i], quad[(i + 1) % 4]);
                cross(a, b, p) >= -1e-3 * side(a, b)
            });
            if inside {
                sum += prob[y * w + x];
                n += 1;
            }
        }
    }
    if n == 0 { 0.0 } else { sum / n as f32 }
}

/// DB's post-processing: one box per region of the `w x h` probability map
/// above 0.3 where the map averages 0.6 or more, grown like DB's unclip.
pub fn text_boxes(prob: &[f32], w: usize, h: usize) -> Vec<Quad> {
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    let mut out = Vec::new();
    let mut regions = 0;
    for start in 0..w * h {
        if seen[start] || prob[start] <= BIN_THRESHOLD {
            continue;
        }
        regions += 1;
        if regions > MAX_REGIONS {
            break;
        }
        // Flood the 8-connected region, keeping each row's first and last
        // pixel: their hull is the region's hull.
        let mut rows: HashMap<usize, (usize, usize)> = HashMap::new();
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let row = rows.entry(y).or_insert((x, x));
            *row = (row.0.min(x), row.1.max(x));
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if !seen[j] && prob[j] > BIN_THRESHOLD {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        let points = rows
            .iter()
            .flat_map(|(&y, &(a, b))| [(a as f32, y as f32), (b as f32, y as f32)])
            .collect();
        let rect = Rect::around(&convex_hull(points));
        if rect.w.min(rect.h) < MIN_SIDE {
            continue;
        }
        if mean_inside(prob, w, h, &rect.corners()) < BOX_THRESHOLD {
            continue;
        }
        let grown = rect.grown(rect.w * rect.h * UNCLIP_RATIO / (2.0 * (rect.w + rect.h)));
        if grown.w.min(grown.h) < MIN_SIDE + 2.0 {
            continue;
        }
        out.push(grown.corners());
    }
    out
}

/// Top to bottom, then left to right among boxes on the same line.
fn reading_order(boxes: &mut [Quad]) {
    boxes.sort_by(|a, b| a[0].1.total_cmp(&b[0].1).then(a[0].0.total_cmp(&b[0].0)));
    for i in 1..boxes.len() {
        let mut j = i;
        while j > 0
            && (boxes[j][0].1 - boxes[j - 1][0].1).abs() < 10.0
            && boxes[j][0].0 < boxes[j - 1][0].0
        {
            boxes.swap(j, j - 1);
            j -= 1;
        }
    }
}

fn bilinear(img: &RgbImage, x: f32, y: f32) -> Rgb<u8> {
    let (w, h) = (img.width() as f32 - 1.0, img.height() as f32 - 1.0);
    let (x, y) = (x.clamp(0.0, w), y.clamp(0.0, h));
    let (x0, y0) = (x.floor(), y.floor());
    let (x1, y1) = ((x0 + 1.0).min(w), (y0 + 1.0).min(h));
    let (fx, fy) = (x - x0, y - y0);
    let at = |x: f32, y: f32| img.get_pixel(x as u32, y as u32).0;
    let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    Rgb(std::array::from_fn(|k| {
        let top = a[k] as f32 * (1.0 - fx) + b[k] as f32 * fx;
        let bottom = c[k] as f32 * (1.0 - fx) + d[k] as f32 * fx;
        (top * (1.0 - fy) + bottom * fy).round() as u8
    }))
}

/// The box cut out of the image and straightened, upright when it is much
/// taller than wide (vertical text).
fn crop(img: &RgbImage, q: &Quad) -> RgbImage {
    let [tl, tr, br, bl] = *q;
    let w = (side(tl, tr).max(side(bl, br)) as u32).max(1);
    let h = (side(tl, bl).max(side(tr, br)) as u32).max(1);
    let out = RgbImage::from_fn(w, h, |x, y| {
        let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
        bilinear(
            img,
            tl.0 + u * (tr.0 - tl.0) + v * (bl.0 - tl.0),
            tl.1 + u * (tr.1 - tl.1) + v * (bl.1 - tl.1),
        )
    });
    if h as f32 / w as f32 >= 1.5 {
        image::imageops::rotate270(&out)
    } else {
        out
    }
}

/// Greedy CTC decoding of `steps x classes` probabilities: the most likely
/// class at each step, repeats collapsed, blanks (class 0) dropped. Class
/// `i` is `dict[i - 1]`, the last class a space. The confidence is the mean
/// probability of the characters kept.
pub fn ctc_decode(probs: &[f32], classes: usize, dict: &[String]) -> (String, f32) {
    let mut text = String::new();
    let (mut sum, mut kept) = (0.0f32, 0usize);
    let mut previous = usize::MAX;
    for step in probs.chunks_exact(classes) {
        let (best, p) =
            step.iter().enumerate().fold(
                (0, f32::MIN),
                |acc, (i, &v)| if v > acc.1 { (i, v) } else { acc },
            );
        if best != 0 && best != previous {
            match dict.get(best - 1) {
                Some(c) => text.push_str(c),
                None => text.push(' '),
            }
            sum += p;
            kept += 1;
        }
        previous = best;
    }
    let confidence = if kept == 0 { 0.0 } else { sum / kept as f32 };
    (text, confidence)
}

pub struct Ocr {
    name: String,
    detector: Pool<Session>,
    recognizer: Pool<Session>,
    dictionary: Vec<String>,
    max_side: u32,
    min_score: f32,
}

impl Ocr {
    pub fn load(
        name: &str,
        detector: &Path,
        recognizer: &Path,
        dictionary: &Path,
        max_side: u32,
        min_score: f32,
        rt: Runtime,
    ) -> Result<Ocr> {
        let yml = std::fs::read_to_string(dictionary)
            .map_err(|e| anyhow!("reading {}: {e}", dictionary.display()))?;
        Ok(Ocr {
            name: name.to_string(),
            detector: open_sessions(detector, rt)?,
            recognizer: open_sessions(recognizer, rt)?,
            dictionary: parse_dictionary(&yml)?,
            max_side,
            min_score,
        })
    }

    /// The probability map of the image as the detector sees it, and its size.
    fn detect(&self, img: &RgbImage) -> Result<(Vec<f32>, u32, u32)> {
        let (dw, dh) = detection_size(img.width(), img.height(), self.max_side);
        let resized = image::imageops::resize(img, dw, dh, FilterType::Triangle);
        let plane = (dw * dh) as usize;
        let mut data = vec![0f32; 3 * plane];
        for (i, p) in resized.pixels().enumerate() {
            let [r, g, b] = p.0;
            for (c, v) in [b, g, r].into_iter().enumerate() {
                data[c * plane + i] = (v as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
        let tensor = Tensor::from_array(([1usize, 3, dh as usize, dw as usize], data))?;
        let mut session = self.detector.get();
        let input = session.inputs()[0].name().to_string();
        let outputs = session.run(ort::inputs![input => tensor])?;
        let (_, map) = outputs[0].try_extract_tensor::<f32>()?;
        if map.len() != plane {
            bail!(
                "{}: the detector returned {} values for {dw}x{dh}",
                self.name,
                map.len()
            );
        }
        Ok((map.to_vec(), dw, dh))
    }

    /// Reads the crops in batches of similar width.
    fn recognize(&self, crops: &[RgbImage]) -> Result<Vec<(String, f32)>> {
        let ratio = |i: &usize| crops[*i].width() as f32 / crops[*i].height() as f32;
        let mut order: Vec<usize> = (0..crops.len()).collect();
        order.sort_by(|a, b| ratio(a).total_cmp(&ratio(b)));
        let mut out = vec![(String::new(), 0.0); crops.len()];
        let height = REC_HEIGHT as usize;
        for batch in order.chunks(REC_BATCH) {
            let widest = batch
                .iter()
                .map(ratio)
                .fold(REC_MIN_WIDTH as f32 / REC_HEIGHT as f32, f32::max);
            let width = ((REC_HEIGHT as f32 * widest) as u32).min(REC_MAX_WIDTH);
            let plane = height * width as usize;
            let mut data = vec![0f32; batch.len() * 3 * plane];
            for (n, &i) in batch.iter().enumerate() {
                let rw = ((REC_HEIGHT as f32 * ratio(&i)).ceil() as u32).clamp(1, width);
                let resized =
                    image::imageops::resize(&crops[i], rw, REC_HEIGHT, FilterType::Triangle);
                for (x, y, p) in resized.enumerate_pixels() {
                    let [r, g, b] = p.0;
                    let at = n * 3 * plane + y as usize * width as usize + x as usize;
                    for (c, v) in [b, g, r].into_iter().enumerate() {
                        data[at + c * plane] = v as f32 / 127.5 - 1.0;
                    }
                }
            }
            let tensor = Tensor::from_array(([batch.len(), 3, height, width as usize], data))?;
            let mut session = self.recognizer.get();
            let input = session.inputs()[0].name().to_string();
            let outputs = session.run(ort::inputs![input => tensor])?;
            let (shape, probs) = outputs[0].try_extract_tensor::<f32>()?;
            let (steps, classes) = (shape[1] as usize, shape[2] as usize);
            if classes != self.dictionary.len() + 2 {
                bail!(
                    "{}: the recognizer has {classes} classes, its dictionary {} characters",
                    self.name,
                    self.dictionary.len()
                );
            }
            for (n, &i) in batch.iter().enumerate() {
                let item = &probs[n * steps * classes..(n + 1) * steps * classes];
                out[i] = ctc_decode(item, classes, &self.dictionary);
            }
        }
        Ok(out)
    }

    /// The text lines of `img` read with enough confidence, top to bottom.
    pub fn read(&self, img: &RgbImage) -> Result<Vec<String>> {
        let (w, h) = (img.width() as f32, img.height() as f32);
        let (map, dw, dh) = self.detect(img)?;
        let (sx, sy) = (w / dw as f32, h / dh as f32);
        let mut boxes: Vec<Quad> = text_boxes(&map, dw as usize, dh as usize)
            .into_iter()
            .map(|q| {
                q.map(|(x, y)| {
                    (
                        (x * sx).round().clamp(0.0, w),
                        (y * sy).round().clamp(0.0, h),
                    )
                })
            })
            .filter(|q| side(q[0], q[1]) as u32 > 3 && side(q[0], q[3]) as u32 > 3)
            .collect();
        reading_order(&mut boxes);
        boxes.truncate(MAX_LINES);
        if boxes.is_empty() {
            return Ok(Vec::new());
        }
        let crops: Vec<RgbImage> = boxes.iter().map(|q| crop(img, q)).collect();
        let read = self.recognize(&crops)?;
        let kept: Vec<(Quad, String)> = boxes
            .into_iter()
            .zip(read)
            .filter(|(_, (text, score))| *score >= self.min_score && !text.trim().is_empty())
            .map(|(q, (text, _))| (q, text))
            .collect();
        Ok(lines(&kept))
    }
}

/// The vertical extent of a box, and where it starts on the left.
fn span(q: &Quad) -> (f32, f32, f32) {
    let ys = q.map(|p| p.1);
    (
        ys.iter().copied().fold(f32::MAX, f32::min),
        ys.iter().copied().fold(f32::MIN, f32::max),
        q[0].0.min(q[3].0),
    )
}

/// Words the detector boxed apart, joined back into lines: a box in reading
/// order continues the line of the box before it when it starts further
/// right and their vertical centers are less than half the smaller height
/// apart.
pub fn lines(boxes: &[(Quad, String)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut previous: Option<(f32, f32, f32)> = None;
    for (q, text) in boxes {
        let (top, bottom, left) = span(q);
        let same = previous.is_some_and(|(t, b, l)| {
            let centers = ((t + b) - (top + bottom)).abs() / 2.0;
            left > l && centers < 0.5 * (b - t).min(bottom - top)
        });
        match (same, out.last_mut()) {
            (true, Some(last)) => {
                last.push(' ');
                last.push_str(text);
            }
            _ => out.push(text.clone()),
        }
        previous = Some((top, bottom, left));
    }
    out
}
