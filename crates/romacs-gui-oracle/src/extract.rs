//! Read a capture back into a grid of cells, using only pixels and the font.
//!
//! Extraction knows the font (file, pixel size) and where the text area
//! starts. It does not know what text, colours or cursor to expect: those are
//! read from pixels so that a spec can be compared against what was found,
//! and so that two captures can be compared with each other (pair mode).

use serde::Serialize;

use crate::capture::Capture;
use crate::glyphs::{FontModel, Style, Template};
use crate::spec::Rgb;

/// Largest per-channel distance from the cell background below which a cell
/// counts as blank. Antialiased fringes of a neighbour's overhang stay well
/// under it; the faintest real glyph stroke measured is far above it.
pub const BLANK_INK: u8 = 48;

/// Template search window, pixels each way. Offsets are reported, and the
/// geometry check fails on any that the declaration does not allow.
pub const SEARCH: i32 = 2;

#[derive(Clone, Debug, Serialize)]
pub struct CellRead {
    pub row: usize,
    pub col: usize,
    /// `' '` for a blank cell.
    pub ch: char,
    pub style: Option<Style>,
    pub score: f32,
    /// Best-scoring different character and its score.
    pub runner_up: Option<(char, f32)>,
    pub dx: i32,
    pub dy: i32,
    /// Most frequent colour in the cell.
    pub bg: Rgb,
    /// Median colour where the matched template is solid.
    pub fg: Option<Rgb>,
    pub underline: bool,
    /// Largest per-channel distance of any pixel from `bg`.
    pub ink: u8,
    /// Observed ink mass over the template's own mass, per style of the
    /// read character. Weight is judged from mass, not shape: rasterisers
    /// disagree on stem shape far more than on how much ink a glyph lays.
    pub mass_ratio: Vec<(Style, f32)>,
    /// Index of the matched template in the font model.
    #[serde(skip)]
    pub template: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Grid {
    pub origin: [i32; 2],
    pub cell_w: i32,
    pub line_h: i32,
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<CellRead>,
}

impl Grid {
    pub fn cell(&self, row: usize, col: usize) -> &CellRead {
        &self.cells[row * self.cols + col]
    }

    pub fn row_text(&self, row: usize) -> String {
        let s: String = (0..self.cols).map(|c| self.cell(row, c).ch).collect();
        s.trim_end().to_string()
    }
}

/// Most frequent colour.
pub fn mode_of(px: &[Rgb]) -> Rgb {
    mode(px)
}

fn mode(px: &[Rgb]) -> Rgb {
    let mut counts: Vec<(Rgb, u32)> = Vec::new();
    for &p in px {
        match counts.iter_mut().find(|(c, _)| *c == p) {
            Some((_, n)) => *n += 1,
            None => counts.push((p, 1)),
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(c, n)| (n, c.0))
        .map(|(c, _)| c)
        .unwrap_or(Rgb([0, 0, 0]))
}

fn sum_dist(a: Rgb, b: Rgb) -> f32 {
    (0..3).map(|i| f32::from(a.0[i].abs_diff(b.0[i]))).sum()
}

struct Obs {
    w: i32,
    h: i32,
    alpha: Vec<f32>,
}

impl Obs {
    fn at(&self, x: i32, y: i32) -> f32 {
        self.alpha[(y * self.w + x) as usize]
    }
}

#[derive(Clone, Copy)]
struct Hit {
    t: usize,
    dx: i32,
    dy: i32,
    score: f32,
}

/// Zero-mean normalised cross-correlation of the observation with one
/// template at one offset, over the cell rectangle.
fn ncc(o: &Obs, so: f32, voo: f32, t: &Template, dx: i32, dy: i32) -> f32 {
    let n = (o.w * o.h) as f32;
    let (mut st, mut stt, mut sot) = (0.0f32, 0.0f32, 0.0f32);
    for j in 0..t.h {
        let y = t.y + dy + j;
        if y < 0 || y >= o.h {
            continue;
        }
        for i in 0..t.w {
            let x = t.x + dx + i;
            if x < 0 || x >= o.w {
                continue;
            }
            let v = t.at(i, j);
            st += v;
            stt += v * v;
            sot += v * o.at(x, y);
        }
    }
    let vtt = stt - st * st / n;
    if vtt <= 1e-6 || voo <= 1e-6 {
        return 0.0;
    }
    (sot - so * st / n) / (voo * vtt).sqrt()
}

fn classify(o: &Obs, model: &FontModel) -> (Option<Hit>, Option<(char, f32)>) {
    let so: f32 = o.alpha.iter().sum();
    let soo: f32 = o.alpha.iter().map(|a| a * a).sum();
    let voo = soo - so * so / (o.w * o.h) as f32;
    let mut best: Option<Hit> = None;
    // Best score per character, for the runner-up.
    let mut per_char: Vec<(char, f32)> = Vec::new();
    for (ti, t) in model.templates.iter().enumerate() {
        let mut tbest = f32::MIN;
        for dy in -SEARCH..=SEARCH {
            for dx in -SEARCH..=SEARCH {
                let s = ncc(o, so, voo, t, dx, dy);
                // Prefer the smaller displacement on a tie, so that an exact
                // placement is never reported as an offset.
                let better = match best {
                    None => true,
                    Some(b) => {
                        s > b.score + 1e-6
                            || ((s - b.score).abs() <= 1e-6
                                && dx.abs() + dy.abs() < b.dx.abs() + b.dy.abs())
                    }
                };
                if better {
                    best = Some(Hit {
                        t: ti,
                        dx,
                        dy,
                        score: s,
                    });
                }
                tbest = tbest.max(s);
            }
        }
        match per_char.iter_mut().find(|(c, _)| *c == t.ch) {
            Some((_, s)) => *s = s.max(tbest),
            None => per_char.push((t.ch, tbest)),
        }
    }
    let runner = best.and_then(|b| {
        let ch = model.templates[b.t].ch;
        per_char
            .into_iter()
            .filter(|(c, _)| *c != ch)
            .max_by(|a, b| a.1.total_cmp(&b.1))
    });
    (best, runner)
}

fn median(mut v: Vec<u8>) -> u8 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// Read one cell. `x0, y0` is its top-left pixel.
pub fn read_cell(cap: &Capture, model: &FontModel, x0: i32, y0: i32) -> CellRead {
    read_cell_masked(cap, model, x0, y0, &[])
}

/// Read one cell, ignoring the pixels flagged in `masked` (row-major over the
/// cell; empty means none). Masked pixels are ink a neighbouring glyph
/// legitimately lays into this cell: italic feet, a `%` wider than its
/// advance.
pub fn read_cell_masked(
    cap: &Capture,
    model: &FontModel,
    x0: i32,
    y0: i32,
    masked: &[bool],
) -> CellRead {
    let (w, h) = (model.cell_w, model.line_h());
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            px.push(cap.get(x0 + x, y0 + y).unwrap_or(Rgb([0, 0, 0])));
        }
    }
    let bg = mode(&px);
    let is_masked = |k: usize| masked.get(k).copied().unwrap_or(false);
    if !masked.is_empty() {
        for (k, p) in px.iter_mut().enumerate() {
            if is_masked(k) {
                *p = bg;
            }
        }
    }
    let ink = px.iter().map(|p| p.dist(bg)).max().unwrap_or(0);
    let blank = CellRead {
        row: 0,
        col: 0,
        ch: ' ',
        style: None,
        score: 1.0,
        runner_up: None,
        dx: 0,
        dy: 0,
        bg,
        fg: None,
        underline: false,
        ink,
        mass_ratio: Vec::new(),
        template: None,
    };
    if ink < BLANK_INK {
        return blank;
    }
    let far = *px
        .iter()
        .max_by(|a, b| sum_dist(**a, bg).total_cmp(&sum_dist(**b, bg)))
        .unwrap_or(&bg);
    let span = sum_dist(far, bg).max(1.0);
    let alpha: Vec<f32> = px
        .iter()
        .map(|p| (sum_dist(*p, bg) / span).min(1.0))
        .collect();
    // Full-width ink rows at or below the baseline are underline candidates.
    let line_rows: Vec<i32> = (model.ascent..h)
        .filter(|&y| {
            (0..w)
                .filter(|&x| alpha[(y * w + x) as usize] >= 0.5)
                .count() as f32
                >= 0.9 * w as f32
        })
        .collect();
    let plain = Obs {
        w,
        h,
        alpha: alpha.clone(),
    };
    let (hit_plain, run_plain) = classify(&plain, model);
    let mut chosen = (hit_plain, run_plain, false);
    if !line_rows.is_empty() {
        let mut masked = alpha.clone();
        for &y in &line_rows {
            for x in 0..w {
                masked[(y * w + x) as usize] = 0.0;
            }
        }
        let rest = masked.iter().fold(0.0f32, |m, &a| m.max(a));
        if rest < 0.2 {
            // Only a line is drawn. It reads as `_` if the underscore
            // template explains it at its own position, else as an
            // underlined blank.
            let underscore = hit_plain
                .filter(|hp| model.templates[hp.t].ch == '_' && hp.dy == 0 && hp.score >= 0.9);
            if underscore.is_none() {
                return CellRead {
                    underline: true,
                    fg: Some(median_color(&px, &alpha, 0.9)),
                    ..blank
                };
            }
        } else {
            let mo = Obs {
                w,
                h,
                alpha: masked,
            };
            let (hit_m, run_m) = classify(&mo, model);
            if let (Some(m), Some(p)) = (hit_m, hit_plain)
                && m.score > p.score
            {
                chosen = (hit_m, run_m, true);
            }
        }
    }
    let (Some(hit), runner, underline) = chosen else {
        return blank;
    };
    let obs_alpha: Vec<f32> = if underline {
        let mut m = alpha.clone();
        for &y in &line_rows {
            for x in 0..w {
                m[(y * w + x) as usize] = 0.0;
            }
        }
        m
    } else {
        alpha.clone()
    };
    let hit = decide_style(
        &Obs {
            w,
            h,
            alpha: obs_alpha.clone(),
        },
        model,
        hit,
    );
    let mass: f32 = obs_alpha.iter().sum();
    let ch = model.templates[hit.t].ch;
    let mut mass_ratio: Vec<(Style, f32)> = Vec::new();
    for t in model
        .templates
        .iter()
        .filter(|t| t.ch == ch && t.hint == model.templates[hit.t].hint)
    {
        mass_ratio.push((t.style, mass / t.cov.iter().sum::<f32>().max(1e-3)));
    }
    let t = &model.templates[hit.t];
    // Foreground, per channel: the third most extreme value, away from the
    // background, over the pixels the matched template covers at least half.
    // Per channel, because romacs antialiases by subpixel: a thin black stem
    // there is `#500300` in one column and `#000753` in the next, and no
    // single pixel is black (measured, corpus-19 `m`). Third, so that one
    // stray pixel cannot set the colour.
    let mut chans: [Vec<u8>; 3] = Default::default();
    for j in 0..t.h {
        for i in 0..t.w {
            let (x, y) = (t.x + hit.dx + i, t.y + hit.dy + j);
            if x < 0 || y < 0 || x >= w || y >= h || t.at(i, j) < 0.5 {
                continue;
            }
            if underline && line_rows.contains(&y) {
                continue;
            }
            let p = px[(y * w + x) as usize];
            for (chan, v) in chans.iter_mut().zip(p.0) {
                chan.push(v);
            }
        }
    }
    let fg = if chans[0].is_empty() {
        far
    } else {
        let pick = |c: usize| {
            let mut v = chans[c].clone();
            v.sort_by_key(|&x| std::cmp::Reverse(x.abs_diff(bg.0[c])));
            v[2.min(v.len() - 1)]
        };
        Rgb([pick(0), pick(1), pick(2)])
    };
    CellRead {
        ch: t.ch,
        style: Some(t.style),
        score: hit.score,
        runner_up: runner,
        dx: hit.dx,
        dy: hit.dy,
        fg: Some(fg),
        underline,
        mass_ratio,
        template: Some(hit.t),
        ..blank
    }
}

/// Pick the style of the already-identified character.
///
/// Slant is a shape difference, so italic is chosen by correlation. Weight is
/// chosen by ink mass: a regular glyph from one rasteriser can correlate
/// better with another rasteriser's bold (measured: GNU PGTK's regular `H`
/// scores 0.917 against the bold template), but bold lays about half as much
/// ink again, and mass survives a change of hinting and gamma.
fn decide_style(o: &Obs, model: &FontModel, hit: Hit) -> Hit {
    let ch = model.templates[hit.t].ch;
    let so: f32 = o.alpha.iter().sum();
    let soo: f32 = o.alpha.iter().map(|a| a * a).sum();
    let voo = soo - so * so / (o.w * o.h) as f32;
    let best_for = |style: Style| -> Option<Hit> {
        let mut b: Option<Hit> = None;
        for (ti, t) in model.templates.iter().enumerate() {
            if t.ch != ch || t.style != style {
                continue;
            }
            for dy in -SEARCH..=SEARCH {
                for dx in -SEARCH..=SEARCH {
                    let s = ncc(o, so, voo, t, dx, dy);
                    let better = b.is_none_or(|b| {
                        s > b.score + 1e-6
                            || ((s - b.score).abs() <= 1e-6
                                && dx.abs() + dy.abs() < b.dx.abs() + b.dy.abs())
                    });
                    if better {
                        b = Some(Hit {
                            t: ti,
                            dx,
                            dy,
                            score: s,
                        });
                    }
                }
            }
        }
        b
    };
    let regular = best_for(Style::Regular);
    let bold = best_for(Style::Bold);
    let italic = best_for(Style::Italic);
    let upright = match (regular, bold) {
        (Some(r), Some(b)) => {
            let mass = |ti: usize| model.templates[ti].cov.iter().sum::<f32>().max(1e-3);
            let (lr, lb) = ((so / mass(r.t)).ln().abs(), (so / mass(b.t)).ln().abs());
            if lb < lr { b } else { r }
        }
        (Some(r), None) => r,
        (None, Some(b)) => b,
        (None, None) => hit,
    };
    match italic {
        Some(i) if i.score > upright.score && regular.is_none_or(|r| i.score > r.score) => i,
        _ => upright,
    }
}

fn median_color(px: &[Rgb], alpha: &[f32], min_alpha: f32) -> Rgb {
    let mut ch: [Vec<u8>; 3] = Default::default();
    for (p, a) in px.iter().zip(alpha) {
        if *a >= min_alpha {
            for (chan, v) in ch.iter_mut().zip(p.0) {
                chan.push(v);
            }
        }
    }
    if ch[0].is_empty() {
        return px[0];
    }
    Rgb([
        median(ch[0].clone()),
        median(ch[1].clone()),
        median(ch[2].clone()),
    ])
}

/// Read every whole cell from `origin` to the capture's edges.
pub fn read_grid(cap: &Capture, model: &FontModel, origin: [i32; 2]) -> Grid {
    let cols = ((cap.width as i32 - origin[0]) / model.cell_w).max(0) as usize;
    let rows = ((cap.height as i32 - origin[1]) / model.line_h()).max(0) as usize;
    read_grid_sized(cap, model, origin, rows, cols)
}

/// Read a `rows` x `cols` grid of cells starting at `origin`.
pub fn read_grid_sized(
    cap: &Capture,
    model: &FontModel,
    origin: [i32; 2],
    rows: usize,
    cols: usize,
) -> Grid {
    let (cw, lh) = (model.cell_w, model.line_h());
    let at = |r: usize, c: usize| (origin[0] + c as i32 * cw, origin[1] + r as i32 * lh);
    let read = |k: usize, masks: Option<&[Vec<bool>]>| {
        let (r, c) = (k / cols, k % cols);
        let (x, y) = at(r, c);
        let mask = masks.map_or(&[][..], |m| &m[k][..]);
        let mut cell = read_cell_masked(cap, model, x, y, mask);
        cell.row = r;
        cell.col = c;
        cell
    };
    let mut cells = par_map(rows * cols, |k| read(k, None));
    // Second pass: mask what the first pass says the left and right
    // neighbours draw into each cell, dilated one pixel sideways because no
    // two rasterisers agree on the last column of an overhang, and re-read.
    let first = cells.clone();
    let mut masks = vec![Vec::new(); rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            let mut mask = vec![false; (cw * lh) as usize];
            let mut any = false;
            for n in [c.wrapping_sub(1), c + 1] {
                if n >= cols {
                    continue;
                }
                let nb = &first[r * cols + n];
                let Some(ti) = nb.template else { continue };
                let t = &model.templates[ti];
                let shift = (n as i32 - c as i32) * cw;
                for j in 0..t.h {
                    for i in 0..t.w {
                        if t.at(i, j) < 0.02 {
                            continue;
                        }
                        let y = t.y + nb.dy + j;
                        let x = t.x + nb.dx + i + shift;
                        for xx in [x - 1, x, x + 1] {
                            if xx >= 0 && xx < cw && y >= 0 && y < lh {
                                mask[(y * cw + xx) as usize] = true;
                                any = true;
                            }
                        }
                    }
                }
            }
            if any {
                masks[r * cols + c] = mask;
            }
        }
    }
    let todo: Vec<usize> = (0..rows * cols).filter(|&k| !masks[k].is_empty()).collect();
    let reread = par_map(todo.len(), |i| read(todo[i], Some(&masks)));
    for (k, cell) in todo.into_iter().zip(reread) {
        cells[k] = cell;
    }
    Grid {
        origin,
        cell_w: cw,
        line_h: lh,
        rows,
        cols,
        cells,
    }
}

/// Correlation of two templates of the same style at their own positions,
/// over one cell. 1.0 means the font draws the two characters identically at
/// this size, so no capture can tell them apart.
pub fn template_similarity(model: &FontModel, a: char, b: char, style: Style) -> Option<f32> {
    let find = |c| {
        model
            .templates
            .iter()
            .find(|t| t.ch == c && t.style == style)
    };
    let (ta, tb) = (find(a)?, find(b)?);
    let (w, h) = (model.cell_w, model.line_h());
    let mut alpha = vec![0.0f32; (w * h) as usize];
    for j in 0..ta.h {
        for i in 0..ta.w {
            let (x, y) = (ta.x + i, ta.y + j);
            if x >= 0 && y >= 0 && x < w && y < h {
                alpha[(y * w + x) as usize] = ta.at(i, j);
            }
        }
    }
    let o = Obs { w, h, alpha };
    let so: f32 = o.alpha.iter().sum();
    let soo: f32 = o.alpha.iter().map(|a| a * a).sum();
    let voo = soo - so * so / (w * h) as f32;
    Some(ncc(&o, so, voo, tb, 0, 0))
}

/// Similarity at or above which two characters count as drawn alike.
pub const EQUIVALENT: f32 = 0.98;

/// Groups of characters the font draws alike in every style, so that no
/// capture can tell them apart (Latin `A` and Greek `Α`, `o` and `ο`).
pub fn equivalence_classes(model: &FontModel, alphabet: &[char]) -> Vec<Vec<char>> {
    let mut classes: Vec<Vec<char>> = Vec::new();
    for &c in alphabet {
        match classes.iter_mut().find(|k| chars_alike(model, k[0], c)) {
            Some(k) => k.push(c),
            None => classes.push(vec![c]),
        }
    }
    classes.retain(|k| k.len() > 1);
    classes
}

/// Whether the font draws `a` and `b` alike in every style it has both in.
pub fn chars_alike(model: &FontModel, a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    let mut compared = false;
    for s in [Style::Regular, Style::Bold, Style::Italic] {
        match (
            template_similarity(model, a, b, s),
            template_similarity(model, b, a, s),
        ) {
            (Some(x), Some(y)) => {
                if x.min(y) < EQUIVALENT {
                    return false;
                }
                compared = true;
            }
            (None, None) => {}
            _ => return false,
        }
    }
    compared
}

/// Whether the font draws `ch` alike in styles `a` and `b`, so that the
/// style cannot be read from pixels (Liberation Mono's italic `=`).
pub fn styles_alike(model: &FontModel, ch: char, a: Style, b: Style) -> bool {
    let find = |s| {
        model
            .templates
            .iter()
            .find(|t| t.ch == ch && t.style == s && t.hint == crate::glyphs::Hint::Light)
    };
    let (Some(ta), Some(tb)) = (find(a), find(b)) else {
        return false;
    };
    let (w, h) = (model.cell_w, model.line_h());
    let render = |t: &Template| {
        let mut v = vec![0.0f32; (w * h) as usize];
        for j in 0..t.h {
            for i in 0..t.w {
                let (x, y) = (t.x + i, t.y + j);
                if x >= 0 && y >= 0 && x < w && y < h {
                    v[(y * w + x) as usize] = t.at(i, j);
                }
            }
        }
        v
    };
    let o = Obs {
        w,
        h,
        alpha: render(ta),
    };
    let so: f32 = o.alpha.iter().sum();
    let soo: f32 = o.alpha.iter().map(|a| a * a).sum();
    let voo = soo - so * so / (w * h) as f32;
    let mass = |t: &Template| t.cov.iter().sum::<f32>();
    ncc(&o, so, voo, tb, 0, 0) >= EQUIVALENT && (mass(ta) / mass(tb).max(1e-3) - 1.0).abs() < 0.05
}

/// Map `f` over `0..n` on scoped threads, preserving order. Cells are
/// independent within a pass, so this changes wall time only.
fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |p| p.get())
        .min(8)
        .min(n.max(1));
    let chunk = n.div_ceil(threads.max(1)).max(1);
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..n)
            .step_by(chunk)
            .map(|start| {
                s.spawn(move || (start..(start + chunk).min(n)).map(f).collect::<Vec<T>>())
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("cell reader panicked"))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::FontSpec;

    /// An observation every placement scores the same on must report the
    /// exact placement, never an offset: an offset is a geometry failure.
    #[test]
    fn ties_prefer_the_exact_placement() {
        let font = FontSpec {
            family: "Liberation Mono".into(),
            pixel_size: 20,
            regular: "/usr/share/fonts/liberation/LiberationMono-Regular.ttf".into(),
            bold: None,
            italic: None,
            ascent: None,
            descent: None,
            cell_width: None,
        };
        let model = FontModel::load(&font, &['H', 'o']).unwrap();
        let (w, h) = (model.cell_w, model.line_h());
        let flat = Obs {
            w,
            h,
            alpha: vec![0.5; (w * h) as usize],
        };
        let (hit, _) = classify(&flat, &model);
        let hit = hit.unwrap();
        assert_eq!((hit.dx, hit.dy), (0, 0));
    }
}
