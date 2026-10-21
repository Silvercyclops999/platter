//! The pictures Platter hands CarPlay when a row isn't a cover: Shuffle my
//! library, Liked songs, the mixes' printed sleeves, and the paper sleeve
//! with the P mark while a cover loads or when it can't. The same drawings
//! the phone uses (paper, brass, the serif), drawn here as PNGs at the size
//! the car shows them.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{imageops, ImageEncoder, Rgb, RgbImage};

use super::{CarAction, CarItem, CarList};
use crate::audio::source::ContextKind;

/// What a row shows of its own.
#[derive(Debug, Clone, PartialEq)]
pub enum CarPicture {
    /// Graphite with the brass shuffle mark.
    Shuffle,
    /// Paper with a brass heart.
    Liked,
    /// A printed sleeve: the mix's name in the serif, its leading artists
    /// under a brass rule.
    Mix { title: String, artists: Vec<String> },
    /// Paper with the P mark: a cover that's loading or didn't come.
    Placeholder,
}

/// The picture a row draws itself, if it has one. A row with a cover shows
/// [`CarPicture::Placeholder`] until the cover comes (see `ios.rs`).
pub fn picture_for(item: &CarItem) -> Option<CarPicture> {
    match &item.action {
        CarAction::ShuffleLibrary => Some(CarPicture::Shuffle),
        CarAction::Open(CarList::Songs { kind: ContextKind::Liked, .. }) => Some(CarPicture::Liked),
        CarAction::Open(CarList::Songs { kind: ContextKind::Mix, title, .. }) if item.thumb.is_none() => Some(CarPicture::Mix {
            title: title.clone(),
            artists: mix_artists(item.subtitle.as_deref().unwrap_or("")),
        }),
        _ => None,
    }
}

/// The leading artists from a mix's "A, B, C and more" line, at most three.
pub fn mix_artists(description: &str) -> Vec<String> {
    let d = description.trim();
    let d = d.strip_prefix("with ").unwrap_or(d);
    let Some(names) = d.strip_suffix(" and more") else { return Vec::new() };
    names.split(", ").map(str::trim).filter(|n| !n.is_empty()).take(3).map(String::from).collect()
}

const PAPER: Rgb<u8> = Rgb([0xED, 0xE6, 0xD6]);
const INK: Rgb<u8> = Rgb([0x1C, 0x17, 0x12]);
const INK_SOFT: Rgb<u8> = Rgb([0x4B, 0x42, 0x37]);
/// Brass printed on paper.
const BRASS_ON_PAPER: Rgb<u8> = Rgb([0x7A, 0x5A, 0x1C]);
/// Brass on graphite.
const BRASS: Rgb<u8> = Rgb([0xCD, 0xAE, 0x6E]);
const GRAPHITE: Rgb<u8> = Rgb([0x21, 0x1F, 0x1C]);
/// The P mark, pressed into the paper.
const MARK: Rgb<u8> = Rgb([0xB9, 0xA5, 0x85]);

/// The serif (Fraunces at its display weight) and the sans (Inter), cut to
/// Latin; both under the SIL Open Font License, alongside in `fonts/`.
static SERIF: &[u8] = include_bytes!("fonts/fraunces-car.ttf");
static SANS: &[u8] = include_bytes!("fonts/inter-car.ttf");

/// Drawn this many times larger, then scaled down, for smooth edges.
const SS: u32 = 3;
/// Below this the sleeve keeps its name only: three names can't be read.
const DETAIL_MIN: u32 = 160;

/// `picture` as a square PNG `px` wide.
pub fn draw(picture: &CarPicture, px: u32) -> Vec<u8> {
    let px = px.max(8);
    let n = px * SS;
    let mut c = Canvas::new(n);
    match picture {
        CarPicture::Shuffle => {
            c.fill(GRAPHITE);
            shuffle_mark(&mut c, BRASS);
        }
        CarPicture::Liked => {
            c.fill(PAPER);
            heart(&mut c, BRASS_ON_PAPER);
        }
        CarPicture::Mix { title, artists } => {
            c.fill(PAPER);
            sleeve(&mut c, title, if px >= DETAIL_MIN { artists } else { &[] });
        }
        CarPicture::Placeholder => {
            c.fill(PAPER);
            c.sheen();
            p_mark(&mut c, MARK);
        }
    }
    let small = imageops::resize(&c.img, px, px, imageops::FilterType::Triangle);
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(small.as_raw(), px, px, image::ExtendedColorType::Rgb8)
        .expect("a PNG of an image in memory");
    out
}

struct Canvas {
    img: RgbImage,
    n: f32,
}

impl Canvas {
    fn new(n: u32) -> Self {
        Canvas { img: RgbImage::new(n, n), n: n as f32 }
    }

    fn fill(&mut self, c: Rgb<u8>) {
        for p in self.img.pixels_mut() {
            *p = c;
        }
    }

    /// Mixes `c` into one pixel by `a` (0 to 1).
    fn blend(&mut self, x: i64, y: i64, c: Rgb<u8>, a: f32) {
        if x < 0 || y < 0 || x >= self.img.width() as i64 || y >= self.img.height() as i64 || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let p = self.img.get_pixel_mut(x as u32, y as u32);
        for i in 0..3 {
            p.0[i] = (p.0[i] as f32 * (1.0 - a) + c.0[i] as f32 * a).round() as u8;
        }
    }

    /// Fills every pixel whose centre `inside` says is in the shape, within
    /// the box (in 0-to-1 units of the picture).
    fn shape(&mut self, (x0, y0, x1, y1): (f32, f32, f32, f32), c: Rgb<u8>, inside: impl Fn(f32, f32) -> bool) {
        let n = self.n;
        let (ax, ay) = ((x0 * n).floor().max(0.0) as i64, (y0 * n).floor().max(0.0) as i64);
        let (bx, by) = ((x1 * n).ceil().min(n) as i64, (y1 * n).ceil().min(n) as i64);
        for y in ay..by {
            for x in ax..bx {
                if inside((x as f32 + 0.5) / n, (y as f32 + 0.5) / n) {
                    self.blend(x, y, c, 1.0);
                }
            }
        }
    }

    /// The light across a paper sleeve, from its top left.
    fn sheen(&mut self) {
        // 160 degrees, as the phone's paper: mostly down, a little right.
        let (dx, dy) = (0.342_f32, 0.940_f32);
        let len = dx.abs() + dy.abs();
        let n = self.n;
        for y in 0..self.img.height() {
            for x in 0..self.img.width() {
                let t = ((x as f32 / n - 0.5) * dx + (y as f32 / n - 0.5) * dy) / len + 0.5;
                if t < 0.6 {
                    self.blend(x as i64, y as i64, Rgb([255, 255, 255]), 0.4 * (1.0 - t / 0.6));
                }
            }
        }
    }

    /// Text in `font` at `size` px with its baseline at `y`, from `x`.
    fn text(&mut self, font: &FontRef, size: f32, x: f32, y: f32, s: &str, c: Rgb<u8>) {
        let f = font.as_scaled(PxScale::from(size));
        let mut pen = x;
        let mut last = None;
        for ch in s.chars() {
            let id = f.glyph_id(ch);
            if let Some(prev) = last {
                pen += f.kern(prev, id);
            }
            let g = id.with_scale_and_position(size, ab_glyph::point(pen, y));
            pen += f.h_advance(id);
            last = Some(id);
            if let Some(o) = font.outline_glyph(g) {
                let b = o.px_bounds();
                o.draw(|gx, gy, a| self.blend(b.min.x as i64 + gx as i64, b.min.y as i64 + gy as i64, c, a));
            }
        }
    }
}

/// How wide `s` is set in `font` at `size`.
fn width(font: &FontRef, size: f32, s: &str) -> f32 {
    let f = font.as_scaled(PxScale::from(size));
    let mut w = 0.0;
    let mut last = None;
    for ch in s.chars() {
        let id = f.glyph_id(ch);
        if let Some(prev) = last {
            w += f.kern(prev, id);
        }
        w += f.h_advance(id);
        last = Some(id);
    }
    w
}

/// Words into lines no wider than `max`.
fn wrap(font: &FontRef, size: f32, s: &str, max: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in s.split_whitespace() {
        match lines.last_mut() {
            Some(l) if width(font, size, &format!("{l} {word}")) <= max => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// Lines as even as they can be without adding one ("Daily / Mix 1", not
/// "Daily Mix / 1").
fn balanced(font: &FontRef, size: f32, s: &str, max: f32) -> Vec<String> {
    let lines = wrap(font, size, s, max);
    let mut best = lines.clone();
    let mut w = max;
    while w > max * 0.4 {
        w -= max * 0.04;
        let tighter = wrap(font, size, s, w);
        if tighter.len() > lines.len() || tighter.iter().any(|l| width(font, size, l) > w + 0.5) {
            break;
        }
        best = tighter;
    }
    best
}

/// `s` cut with an ellipsis to fit `max`.
fn fit(font: &FontRef, size: f32, s: &str, max: f32) -> String {
    if width(font, size, s) <= max {
        return s.to_string();
    }
    let mut cut: String = s.to_string();
    while !cut.is_empty() && width(font, size, &format!("{}…", cut.trim_end())) > max {
        cut.pop();
    }
    format!("{}…", cut.trim_end())
}

/// The printed sleeve: as `.mix-art--sleeve` on the phone and the computer.
fn sleeve(c: &mut Canvas, title: &str, artists: &[String]) {
    let serif = FontRef::try_from_slice(SERIF).expect("the serif");
    let sans = FontRef::try_from_slice(SANS).expect("the sans");
    let n = c.n;
    let pad = n * 0.085;
    let inner = n - pad * 2.0;
    let detail = !artists.is_empty();
    // The name: a fifth of the sleeve, a little larger with nothing under it.
    let mut size = inner * if detail { 0.2 } else { 0.26 };
    // A word that can't fit on a line sets the whole name smaller.
    while size > inner * 0.12 && title.split_whitespace().any(|w| width(&serif, size, w) > inner) {
        size *= 0.92;
    }
    let max_lines = if detail { 3 } else { 2 };
    let mut lines = balanced(&serif, size, title, inner);
    if lines.len() > max_lines {
        let rest = lines.split_off(max_lines - 1).join(" ");
        lines.push(fit(&serif, size, &rest, inner));
    }
    let ascent = serif.as_scaled(PxScale::from(size)).ascent();
    let line_h = size;
    let mut y = if detail { pad } else { n - pad - line_h * lines.len() as f32 };
    for l in &lines {
        c.text(&serif, size, pad, y + ascent * 0.92, l, INK);
        y += line_h;
    }
    if detail {
        // From the foot up: the names, then the brass rule over them.
        let small = (inner * 0.095).max(11.0 * SS as f32);
        let gap = inner * 0.017;
        let names_h = small * 1.2 * artists.len() as f32 + gap * (artists.len() as f32 - 1.0);
        let rule_y = n - pad - names_h - inner * 0.052;
        let t = (n / 140.0).max(1.0);
        c.shape((pad / n, rule_y / n, (n - pad) / n, (rule_y + t) / n), BRASS_ON_PAPER, |_, _| true);
        let asc = sans.as_scaled(PxScale::from(small)).ascent();
        let mut y = rule_y + t + inner * 0.052;
        for a in artists {
            c.text(&sans, small, pad, y + asc * 0.95, &fit(&sans, small, a, inner), INK_SOFT);
            y += small * 1.2 + gap;
        }
    }
}

/// The P mark, a third of the sleeve wide, in the middle.
fn p_mark(c: &mut Canvas, col: Rgb<u8>) {
    // The mark's own box (296..776 by 216..876), set 30% of the sleeve wide.
    let scale = 0.3 / 480.0;
    let (ox, oy) = (0.5 - 240.0 * scale, 0.5 - 330.0 * scale);
    let to = |x: f32, y: f32| ((x - ox) / scale + 296.0, (y - oy) / scale + 216.0);
    let bounds = (ox, oy, ox + 480.0 * scale, oy + 660.0 * scale);
    c.shape(bounds, col, |x, y| {
        let (u, v) = to(x, y);
        let d = ((u - 536.0).powi(2) + (v - 456.0).powi(2)).sqrt();
        let stem = (296.0..=408.0).contains(&u) && (456.0..=876.0).contains(&v);
        (d <= 240.0 || stem) && d > 80.0
    });
}

/// A cubic from `a` to `d`, as points along it.
fn cubic(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32), out: &mut Vec<(f32, f32)>) {
    for i in 1..=16 {
        let t = i as f32 / 16.0;
        let u = 1.0 - t;
        out.push((
            u * u * u * a.0 + 3.0 * u * u * t * b.0 + 3.0 * u * t * t * c.0 + t * t * t * d.0,
            u * u * u * a.1 + 3.0 * u * u * t * b.1 + 3.0 * u * t * t * c.1 + t * t * t * d.1,
        ));
    }
}

/// Whether a point is inside a closed outline (even-odd).
fn inside_polygon(p: (f32, f32), poly: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The heart, filled, about a third of the sleeve: the phone's Liked songs heart.
fn heart(c: &mut Canvas, col: Rgb<u8>) {
    // On a 24-unit grid, as the phone's icon.
    let mut h = vec![(12.0, 21.0)];
    cubic((12.0, 21.0), (12.0, 21.0), (4.5, 16.4), (2.4, 11.8), &mut h);
    cubic((2.4, 11.8), (1.0, 8.6), (3.0, 5.0), (6.5, 5.0), &mut h);
    cubic((6.5, 5.0), (8.5, 5.0), (10.1, 6.1), (11.0, 7.6), &mut h);
    cubic((11.0, 7.6), (11.9, 6.1), (13.5, 5.0), (15.5, 5.0), &mut h);
    cubic((15.5, 5.0), (19.0, 5.0), (21.0, 8.6), (19.6, 11.8), &mut h);
    cubic((19.6, 11.8), (17.5, 16.4), (12.0, 21.0), (12.0, 21.0), &mut h);
    let side = 0.36;
    let o = 0.5 - side / 2.0;
    let poly: Vec<(f32, f32)> = h.iter().map(|&(x, y)| (o + x / 24.0 * side, o + (y + 0.5) / 24.0 * side)).collect();
    c.shape((o, o, o + side, o + side), col, |x, y| inside_polygon((x, y), &poly));
}

/// The distance from `p` to the segment `a`-`b`.
fn to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx * dx + dy * dy;
    let t = if len == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len).clamp(0.0, 1.0) };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// The shuffle mark, stroked: two crossing paths with their arrows.
fn shuffle_mark(c: &mut Canvas, col: Rgb<u8>) {
    let mut lines: Vec<Vec<(f32, f32)>> = vec![
        vec![(18.0, 14.0), (22.0, 18.0), (18.0, 22.0)],
        vec![(18.0, 2.0), (22.0, 6.0), (18.0, 10.0)],
    ];
    let mut a = vec![(2.0, 18.0), (3.4, 18.0)];
    cubic((3.4, 18.0), (4.7, 18.0), (5.9, 17.4), (6.7, 16.3), &mut a);
    a.push((12.8, 7.7));
    cubic((12.8, 7.7), (13.5, 6.6), (14.8, 6.0), (16.1, 6.0), &mut a);
    a.push((22.0, 6.0));
    let mut b = vec![(2.0, 6.0), (3.9, 6.0)];
    cubic((3.9, 6.0), (5.4, 6.0), (6.8, 6.9), (7.5, 8.2), &mut b);
    let mut d = vec![(22.0, 18.0), (16.1, 18.0)];
    cubic((16.1, 18.0), (14.8, 18.0), (13.5, 17.3), (12.8, 16.2), &mut d);
    d.push((12.3, 15.4));
    lines.extend([a, b, d]);
    let side = 0.44;
    let o = 0.5 - side / 2.0;
    let pts: Vec<Vec<(f32, f32)>> = lines.iter().map(|l| l.iter().map(|&(x, y)| (o + x / 24.0 * side, o + y / 24.0 * side)).collect()).collect();
    let half = 1.0 / 24.0 * side;
    c.shape((o - half, o - half, o + side + half, o + side + half), col, |x, y| {
        pts.iter().any(|l| l.windows(2).any(|w| to_segment((x, y), w[0], w[1]) <= half))
    });
}
