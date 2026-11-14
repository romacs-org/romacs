//! The Romacs mark, a red brush R with a blue crescent
//! (crates/romacs-display-runtime/assets/window-icon.svg), over the
//! wordmark's Iosevka ExtraBold outlines (assets/wordmark.svg), drawn as
//! braille cells: each cell's 2 by 4 dots are those the outlines cover,
//! filled even-odd.  A band of light sweeps across the letters every
//! [`SWEEP`] seconds and faint dots drift up behind them.  Braille
//! rasterising follows drawille's dot layout (asciimoo/drawille, MIT); no
//! code is taken from it.
//!
//! The *Romacs* landing (lisp/romacs/romacs-landing.el) shows a still frame from
//! `romacs--logo-frame`, the render thread draws its motion, and the GUI
//! host draws it while a first start prepares its world.
use std::sync::{Arc, Mutex, OnceLock};

/// One cell: its character and, unless it is empty, its colour as RGB.
pub type Cell = (char, Option<u32>);

/// Seconds between the starts of two sweeps of light.
pub const SWEEP: f64 = 6.0;

const WORD: u32 = 0xd8d8d8;
const MARK: &str = include_str!("../../romacs-display-runtime/assets/window-icon.svg");
const WORDMARK: &str = include_str!("../../../assets/wordmark.svg");

/// What a frame shows: COLS by ROWS cells, each CELL_ASPECT as wide as it
/// is high.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub cols: usize,
    pub rows: usize,
    pub cell_aspect: f64,
}

impl View {
    /// 1..=400 columns, 1..=200 rows, a cell aspect in (0, 2].
    pub fn valid(&self) -> bool {
        (1..=400).contains(&self.cols) && (1..=200).contains(&self.rows)
            && self.cell_aspect.is_finite() && self.cell_aspect > 0.0 && self.cell_aspect <= 2.0
    }
}

/// A closed outline's points.
type Polygon = Vec<(f64, f64)>;

/// The outlines of an SVG path's absolute M, L, H, V, Q, C and Z commands
/// (all the mark and the wordmark use), curves cut into straight pieces.
fn outlines(d: &str) -> Vec<Polygon> {
    let mut tokens = vec![];
    let mut number = String::new();
    let flush = |number: &mut String, tokens: &mut Vec<Token>| {
        if !number.is_empty() { tokens.push(Token::Number(number.parse().unwrap_or(0.0))); number.clear(); }
    };
    for c in d.chars() {
        match c {
            'A'..='Z' | 'a'..='z' if c != 'e' => { flush(&mut number, &mut tokens); tokens.push(Token::Command(c)); }
            '-' if !number.is_empty() && !number.ends_with('e') => { flush(&mut number, &mut tokens); number.push(c); }
            '0'..='9' | '.' | '-' | 'e' => number.push(c),
            _ => flush(&mut number, &mut tokens),
        }
    }
    flush(&mut number, &mut tokens);
    let mut polygons: Vec<Polygon> = vec![];
    let (mut command, mut at, mut i) = ('M', (0.0, 0.0), 0);
    let take = |i: &mut usize| match tokens.get(*i) { Some(Token::Number(n)) => { *i += 1; Some(*n) } _ => None };
    while i < tokens.len() {
        if let Token::Command(c) = tokens[i] { command = c; i += 1; if c == 'Z' || c == 'z' { if let Some(p) = polygons.last().and_then(|p| p.first().copied()) { at = p; } continue; } }
        let start = i;
        match command {
            'M' => { let (Some(x), Some(y)) = (take(&mut i), take(&mut i)) else { break }; at = (x, y); polygons.push(vec![at]); command = 'L'; }
            'L' => { let (Some(x), Some(y)) = (take(&mut i), take(&mut i)) else { break }; at = (x, y); }
            'H' => { let Some(x) = take(&mut i) else { break }; at.0 = x; }
            'V' => { let Some(y) = take(&mut i) else { break }; at.1 = y; }
            'Q' | 'C' => {
                let n = if command == 'Q' { 2 } else { 3 };
                let mut points = vec![at];
                for _ in 0..n { let (Some(x), Some(y)) = (take(&mut i), take(&mut i)) else { break }; points.push((x, y)); }
                if points.len() != n + 1 { break; }
                if let Some(polygon) = polygons.last_mut() {
                    for step in 1..12 { polygon.push(bezier(&points, f64::from(step) / 12.0)); }
                }
                at = points[n];
            }
            _ => break,
        }
        if i == start { break; }
        if let Some(polygon) = polygons.last_mut() { polygon.push(at); }
    }
    polygons
}

#[derive(Clone, Copy)]
enum Token { Command(char), Number(f64) }

fn bezier(points: &[(f64, f64)], t: f64) -> (f64, f64) {
    let mut p = points.to_vec();
    while p.len() > 1 {
        p = p.windows(2).map(|w| (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t)).collect();
    }
    p[0]
}

/// Each `<path>` of SVG in order: its `fill` and its `d`.
fn path_data(svg: &str) -> Vec<(Option<u32>, &str)> {
    svg.split("<path").skip(1).map(|element| {
        let attribute = |name: &str| element.split(&format!("{name}=\"")).nth(1).and_then(|rest| rest.split('"').next());
        let fill = attribute("fill").and_then(|f| f.strip_prefix('#')).and_then(|hex| u32::from_str_radix(hex, 16).ok());
        (fill, attribute("d").unwrap_or_default())
    }).collect()
}

/// Each `d="..."` of SVG, with the `fill` before it, in order.
fn paths(svg: &str) -> Vec<(Option<u32>, Vec<Polygon>)> {
    path_data(svg).into_iter().map(|(fill, d)| (fill, outlines(d))).collect()
}

/// The mark's SVG paths, each its colour and `d`: the R (with its counter,
/// filled even-odd), then the crescent.
pub fn mark_paths() -> Vec<(u32, &'static str)> {
    path_data(MARK).into_iter().filter_map(|(fill, d)| Some((fill?, d))).collect()
}

/// The wordmark's SVG paths' `d`, in the wordmark's own units.
pub fn wordmark_paths() -> Vec<&'static str> {
    path_data(WORDMARK).into_iter().map(|(_, d)| d).collect()
}

/// The wordmark's colour.
pub const WORDMARK_COLOUR: u32 = WORD;

/// Where a view puts the mark and the wordmark: each `(x, y, width, height)`
/// in cell heights from the view's top left (a cell is `cell_aspect` wide).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub mark: (f64, f64, f64, f64),
    pub word: (f64, f64, f64, f64),
}

/// The mark's filled shapes, each its colour and outlines (the R, then the
/// crescent), and the wordmark's outlines.
type Shapes = (Vec<(u32, Vec<Polygon>)>, Vec<Polygon>);

/// The shapes drawn.
fn shapes() -> &'static Shapes {
    static SHAPES: OnceLock<Shapes> = OnceLock::new();
    SHAPES.get_or_init(|| {
        let mark = paths(MARK).into_iter().filter_map(|(fill, polygons)| Some((fill?, polygons))).collect();
        let word = paths(WORDMARK).into_iter().flat_map(|(_, polygons)| polygons).collect();
        (mark, word)
    })
}

fn bounds(polygons: &[Polygon]) -> (f64, f64, f64, f64) {
    polygons.iter().flatten().fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)))
}

/// Where a frame's dots are lit: per cell its braille bits and the colour
/// of the shape most of them belong to (0 none).
struct Layout {
    view: View,
    cells: Vec<(u8, u32)>,
    /// The lit cells' extent: first and last column and row.
    extent: (f64, f64, f64, f64),
}

/// VIEW's layout, from a small cache: a frame of the same view reuses it.
fn layout(view: View) -> Arc<Layout> {
    static CACHE: Mutex<Vec<Arc<Layout>>> = Mutex::new(Vec::new());
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = cache.iter().find(|l| l.view == view) { return found.clone(); }
    let made = Arc::new(lay_out(view));
    cache.insert(0, made.clone());
    cache.truncate(4);
    made
}

/// A box: least x and y, greatest x and y.
type Bounds = (f64, f64, f64, f64);

/// The outlines' bounds: the mark's, then the wordmark's.
fn shape_bounds() -> (Bounds, Bounds) {
    let (mark, word) = shapes();
    (bounds(&mark.iter().flat_map(|(_, p)| p.iter().cloned()).collect::<Vec<_>>()), bounds(word))
}

/// The mark centred in the upper part of VIEW, the wordmark under it.
pub fn placement(view: View) -> Placement {
    let (mark_bounds, word_bounds) = shape_bounds();
    let (width, height) = (view.cols as f64 * view.cell_aspect, view.rows as f64);
    let (mw, mh) = (mark_bounds.2 - mark_bounds.0, mark_bounds.3 - mark_bounds.1);
    let (ww, wh) = (word_bounds.2 - word_bounds.0, word_bounds.3 - word_bounds.1);
    // Heights in cells: the mark, a gap, the wordmark; within the width.
    let mut mark_h = height * 0.66;
    mark_h = mark_h.min(width * 0.85 * mh / mw);
    let word_h = (mark_h * 0.2).min(width * 0.6 * wh / ww);
    let gap = mark_h * 0.12;
    let top = (height - (mark_h + gap + word_h)) / 2.0;
    let centred = |w: f64, h: f64, y: f64| ((width - w) / 2.0, y, w, h);
    Placement {
        mark: centred(mark_h * mw / mh, mark_h, top),
        word: centred(word_h * ww / wh, word_h, top + mark_h + gap),
    }
}

/// VIEW's placement rasterised to dots: dot (i, j) lies at x (i + .5) / 2 and y (j + .5) / 4
/// cells, a cell being CELL_ASPECT wide and 1 high.
fn lay_out(view: View) -> Layout {
    let (mark, word) = shapes();
    let (mark_bounds, word_bounds) = shape_bounds();
    let placed = placement(view);
    let place = |bounds: (f64, f64, f64, f64), (x, y, _, h): (f64, f64, f64, f64)| {
        let scale = h / (bounds.3 - bounds.1);
        move |(px, py): (f64, f64)| (x + (px - bounds.0) * scale, y + (py - bounds.1) * scale)
    };
    let mut drawn: Vec<(u32, Vec<Polygon>)> = mark.iter().map(|(colour, polygons)| {
        let at = place(mark_bounds, placed.mark);
        (*colour, polygons.iter().map(|p| p.iter().map(|&q| at(q)).collect()).collect())
    }).collect();
    let at = place(word_bounds, placed.word);
    drawn.push((WORD, word.iter().map(|p| p.iter().map(|&q| at(q)).collect()).collect()));
    let (dots_w, dots_h) = (view.cols * 2, view.rows * 4);
    let mut dots = vec![0u32; dots_w * dots_h];
    for (colour, polygons) in &drawn {
        for j in 0..dots_h {
            let y = (j as f64 + 0.5) / 4.0;
            let mut crossings: Vec<f64> = polygons.iter().flat_map(|p| p.iter().zip(p.iter().cycle().skip(1)))
                .filter(|(a, b)| (a.1 <= y) != (b.1 <= y))
                .map(|(a, b)| a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0))
                .collect();
            crossings.sort_by(f64::total_cmp);
            for span in crossings.chunks_exact(2) {
                let first = ((span[0] / view.cell_aspect * 2.0) - 0.5).ceil().max(0.0) as usize;
                let last = ((span[1] / view.cell_aspect * 2.0) - 0.5).floor();
                if last < 0.0 { continue; }
                for i in first..=(last as usize).min(dots_w - 1) { dots[j * dots_w + i] = *colour; }
            }
        }
    }
    let cells = (0..view.rows * view.cols).map(|cell| {
        let (row, col) = (cell / view.cols, cell % view.cols);
        let mut bits = 0u8;
        let mut votes: Vec<(u32, u32)> = vec![];
        for (dx, dy, bit) in BRAILLE {
            let colour = dots[(row * 4 + dy) * dots_w + col * 2 + dx];
            if colour != 0 {
                bits |= bit;
                match votes.iter_mut().find(|(c, _)| *c == colour) { Some((_, n)) => *n += 1, None => votes.push((colour, 1)) }
            }
        }
        (bits, votes.iter().max_by_key(|(_, n)| *n).map_or(0, |(c, _)| *c))
    }).collect::<Vec<_>>();
    let lit = cells.iter().enumerate().filter(|(_, (bits, _))| *bits != 0).map(|(i, _)| ((i % view.cols) as f64, (i / view.cols) as f64));
    let extent = lit.fold((f64::MAX, f64::MIN, f64::MAX, f64::MIN), |(c0, c1, r0, r1), (c, r)| (c0.min(c), c1.max(c), r0.min(r), r1.max(r)));
    Layout { view, cells, extent }
}

/// A braille cell's dots, column, row and bit (drawille's layout).
const BRAILLE: [(usize, usize, u8); 8] =
    [(0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (1, 0, 0x08), (1, 1, 0x10), (1, 2, 0x20), (0, 3, 0x40), (1, 3, 0x80)];

/// VIEW's frame T seconds into its motion, row by row: the letters lit
/// by the sweep, faint dots drifting up through the empty cells.  VIEW
/// must be [`View::valid`].
pub fn frame(view: View, t: f64) -> Vec<Cell> {
    let layout = layout(view);
    // The band runs across the letters' extent, not the whole view.
    let (c0, c1, r0, r1) = layout.extent;
    let (width, height) = (((c1 - c0).max(1.0)) * view.cell_aspect, (r1 - r0).max(1.0));
    // The band crosses the diagonal once a period, briefly off its ends.
    let band = (t / SWEEP).fract() * 1.2 - 0.1;
    let mut cells: Vec<Cell> = layout.cells.iter().enumerate().map(|(i, &(bits, colour))| {
        if bits == 0 { return (' ', None); }
        let (row, col) = ((i / view.cols) as f64 - r0, (i % view.cols) as f64 - c0);
        let along = (col * view.cell_aspect + 0.55 * row) / (width + 0.55 * height);
        let light = (-((along - band) / 0.11).powi(2)).exp();
        let light = (light * 6.0).round() / 6.0;
        (char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' '), Some(lit(colour, light)))
    }).collect();
    let count = (view.cols * view.rows / 30).max(6);
    let (dots_w, dots_h) = (view.cols * 2, view.rows * 4);
    for particle in 0..count {
        let (a, b, c) = (hash(particle, 1), hash(particle, 2), hash(particle, 3));
        let x = (a * dots_w as f64) as usize % dots_w;
        let rise = (c + t * (3.0 + 6.0 * b) / dots_h as f64).fract();
        let y = dots_h - 1 - ((rise * dots_h as f64) as usize).min(dots_h - 1);
        let cell = (y / 4) * view.cols + x / 2;
        if cells[cell].1.is_some() && layout.cells[cell].0 != 0 { continue; }
        let bit = BRAILLE.iter().find(|(dx, dy, _)| *dx == x % 2 && *dy == y % 4).map_or(0, |b| b.2);
        let bits = cells[cell].0 as u32 - if cells[cell].0 == ' ' { 0x20 } else { 0x2800 };
        let grey = 0x30 + (b * 3.0).floor() as u32 * 0x10;
        cells[cell] = (char::from_u32(0x2800 + (bits | u32::from(bit))).unwrap_or(' '), Some(grey * 0x010101));
    }
    cells
}

/// COLOUR raised toward white by LIGHT (0 to 1).
fn lit(colour: u32, light: f64) -> u32 {
    let lift = 0.8 * light * light;
    [16, 8, 0].iter().fold(0, |rgb, &shift| {
        let channel = f64::from((colour >> shift) & 0xff);
        rgb | (((channel + (255.0 - channel) * lift).round() as u32).min(255) << shift)
    })
}

/// A fixed pseudo-random number in [0, 1) for PARTICLE's facet SALT.
fn hash(particle: usize, salt: u64) -> f64 {
    let mut x = (particle as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ salt.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 31;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// CELLS as rows of COLS joined by newlines.
pub fn text(cells: &[Cell], cols: usize) -> String {
    let mut out = String::with_capacity(cells.len() * 3 + cells.len() / cols);
    for (i, (c, _)) in cells.iter().enumerate() {
        if i > 0 && i % cols == 0 { out.push('\n'); }
        out.push(*c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mark's two fills, as window-icon.svg gives them.
    const RED: u32 = 0xff5f59;
    const BLUE: u32 = 0x2fafff;

    fn view() -> View { View { cols: 120, rows: 40, cell_aspect: 0.5 } }

    #[test]
    fn the_mark_and_the_wordmark_are_read() {
        let (mark, word) = shapes();
        assert_eq!(mark.iter().map(|(c, _)| *c).collect::<Vec<_>>(), [RED, BLUE]);
        assert_eq!(mark[0].1.len(), 2, "the R and its counter");
        assert!(word.len() >= 6, "romacs: {} outlines", word.len());
    }

    #[test]
    fn the_frame_draws_the_r_crescent_and_wordmark_at_rest() {
        let cells = frame(view(), 0.0);
        assert_eq!(cells.len(), 120 * 40);
        let count = |c: u32| cells.iter().filter(|(_, colour)| *colour == Some(c)).count();
        assert!(count(RED) > 300, "red cells {}", count(RED));
        assert!(count(BLUE) > 40, "blue cells {}", count(BLUE));
        assert!(count(WORD) > 60, "wordmark cells {}", count(WORD));
        assert!(cells.iter().all(|(c, colour)| (*c == ' ') == colour.is_none()));
    }

    #[test]
    fn the_light_moves_and_the_frame_repeats() {
        let at = |t| frame(view(), t);
        assert_ne!(at(0.0), at(SWEEP * 0.4));
        let lit = at(SWEEP * 0.4).iter().filter(|(_, c)| c.is_some_and(|c| c != RED && c != BLUE && c != WORD && (c >> 16) > 0x60)).count();
        assert!(lit > 20, "cells in the band {lit}");
    }

    #[test]
    fn text_joins_rows() {
        assert_eq!(text(&[('a', None), ('b', None), ('c', None), ('d', None)], 2), "ab\ncd");
    }
}
