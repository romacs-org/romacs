//! Pair mode: compare what two captures of one scenario show.
//!
//! This is what replaces a GNU-against-romacs exact-display assertion: run
//! the scenario in both editors, read both captures with the same font and
//! grid, and compare the extracted structure (cell text, style, underline,
//! colours, glyph offsets, cursor, fringes), never the pixels: cairo and
//! wgpu do not agree on antialiasing, and need not.
//!
//! Scope: one window, one font, monospace cells. Variable pitch, images,
//! multiple windows and the mode line are not compared yet.

use serde::Serialize;

use crate::capture::Capture;
use crate::check::{TOL_BG, TOL_FG};
use crate::extract::{Grid, chars_alike, mode_of, read_grid_sized, styles_alike};
use crate::glyphs::{FontModel, alphabet};
use crate::spec::Spec;

#[derive(Clone, Debug, Serialize)]
pub struct PairDiff {
    pub row: Option<usize>,
    pub col: Option<usize>,
    pub what: String,
}

/// The two grids of one scenario, read identically.
pub struct Pair {
    pub a: Grid,
    pub b: Grid,
    pub diffs: Vec<PairDiff>,
}

/// Read both captures with the scenario's font and grid and compare them.
/// The spec's `expect.rows` are not used: only its frame, font, origin,
/// column count and fringe width.
pub fn pair(spec: &Spec, a: &Capture, b: &Capture) -> Result<Pair, String> {
    let model = FontModel::load(&spec.font, &alphabet([]))?;
    let (cw, lh) = (model.cell_w, model.line_h());
    let fringe = spec.frame.fringe as i32;
    let exp = spec.expect.as_ref();
    let [ox, oy] = exp.map_or([fringe, 0], |e| e.origin);
    if (a.width, a.height) != (b.width, b.height) {
        return Err(format!(
            "{}: captures differ in size, {}x{} vs {}x{}",
            spec.name, a.width, a.height, b.width, b.height
        ));
    }
    let mut diffs = Vec::new();
    let (w, h) = (a.width as i32, a.height as i32);
    let cols = exp
        .and_then(|e| e.cols)
        .unwrap_or_else(|| ((w - fringe - ox) / cw).max(0) as usize);
    let rows = ((h - oy) / lh).max(0) as usize;
    let ga = read_grid_sized(a, &model, [ox, oy], rows, cols);
    let gb = read_grid_sized(b, &model, [ox, oy], rows, cols);
    diffs.extend(compare(&model, &ga, &gb));
    diffs.extend(compare_outside(a, b, [ox, oy], rows, cols, cw, lh, fringe));
    Ok(Pair {
        a: ga,
        b: gb,
        diffs,
    })
}

/// Cell-by-cell comparison of two grids read with the same model.
pub fn compare(model: &FontModel, a: &Grid, b: &Grid) -> Vec<PairDiff> {
    let mut out = Vec::new();
    let mut diff = |r: usize, c: usize, what: String| {
        out.push(PairDiff {
            row: Some(r),
            col: Some(c),
            what,
        })
    };
    for r in 0..a.rows.min(b.rows) {
        for c in 0..a.cols.min(b.cols) {
            let (x, y) = (a.cell(r, c), b.cell(r, c));
            if !chars_alike(model, x.ch, y.ch) {
                diff(r, c, format!("text {:?} vs {:?}", x.ch, y.ch));
                continue;
            }
            if x.bg.dist(y.bg) > TOL_BG {
                diff(r, c, format!("background {} vs {}", x.bg, y.bg));
            }
            if x.underline != y.underline {
                diff(
                    r,
                    c,
                    format!("underline {} vs {}", x.underline, y.underline),
                );
            }
            if x.ch == ' ' {
                continue;
            }
            // Same bound as spec mode: GNU and romacs foregrounds read from
            // the 20-scene corpus differ by at most 5 levels (1,969 of 2,045
            // cells by 0), so 8 still separates an 11-level change.
            if let (Some(fx), Some(fy)) = (x.fg, y.fg)
                && fx.dist(fy) > TOL_FG
            {
                diff(r, c, format!("foreground {fx} vs {fy}"));
            }
            if let (Some(sx), Some(sy)) = (x.style, y.style)
                && sx != sy
                && !styles_alike(model, x.ch, sx, sy)
            {
                diff(r, c, format!("style {sx:?} vs {sy:?}"));
            }
            if (x.dx, x.dy) != (y.dx, y.dy) {
                diff(
                    r,
                    c,
                    format!("glyph offset ({}, {}) vs ({}, {})", x.dx, x.dy, y.dx, y.dy),
                );
            }
        }
    }
    out
}

/// Fringes and the remainder of the frame: compare the background drawn
/// there, and where bitmaps or other ink appear, per row band.
#[allow(clippy::too_many_arguments)]
fn compare_outside(
    a: &Capture,
    b: &Capture,
    origin: [i32; 2],
    rows: usize,
    cols: usize,
    cw: i32,
    lh: i32,
    fringe: i32,
) -> Vec<PairDiff> {
    let (w, h) = (a.width as i32, a.height as i32);
    let [ox, oy] = origin;
    let gx1 = ox + cols as i32 * cw;
    let mut regions: Vec<(&str, std::ops::Range<i32>)> = Vec::new();
    if fringe > 0 {
        regions.push(("left fringe", 0..fringe));
        regions.push(("right fringe", w - fringe..w));
    }
    if gx1 < w - fringe {
        regions.push(("right of the grid", gx1..w - fringe));
    }
    let mut out = Vec::new();
    for (name, xs) in regions {
        let band = |cap: &Capture, y0: i32, y1: i32| -> Vec<crate::spec::Rgb> {
            (y0..y1)
                .flat_map(|y| xs.clone().map(move |x| (x, y)))
                .filter_map(|(x, y)| cap.get(x, y))
                .collect()
        };
        let (ba, bb) = (mode_of(&band(a, 0, h)), mode_of(&band(b, 0, h)));
        if ba.dist(bb) > TOL_BG {
            out.push(PairDiff {
                row: None,
                col: None,
                what: format!("{name} background {ba} vs {bb}"),
            });
        }
        for r in 0..=rows {
            let (y0, y1) = if r < rows {
                (oy + r as i32 * lh, oy + (r as i32 + 1) * lh)
            } else {
                (oy + rows as i32 * lh, h)
            };
            let ia = band(a, y0, y1).iter().any(|p| p.dist(ba) > TOL_BG);
            let ib = band(b, y0, y1).iter().any(|p| p.dist(bb) > TOL_BG);
            if ia != ib {
                out.push(PairDiff {
                    row: Some(r),
                    col: None,
                    what: format!("{name}: ink {ia} vs {ib}"),
                });
            }
        }
    }
    out
}
