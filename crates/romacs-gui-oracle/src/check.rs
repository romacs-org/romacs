//! Spec mode: assert a capture against a declared scene.
//!
//! Every assertion is made from the capture and the declaration alone:
//!
//! | kind | asserted from pixels |
//! |---|---|
//! | `Text` | each cell's character, read as an argmax over the alphabet |
//! | `Colour` | cell background (mode) and glyph foreground |
//! | `Style` | weight (ink mass), slant (shape), underline (full-width line) |
//! | `Geometry` | offset of each glyph's best template placement |
//! | `Cursor` | the declared cell, and no other, has the cursor colour as background |
//! | `Stray` | pixels outside the text grid and fringes are frame background |
//! | `Fringe` | fringe background, and bitmaps exactly on the declared rows |
//!
//! Wrapping is asserted through `Text`: a declared row is the exact text the
//! row must show, so a break one column off misreads two rows.

use serde::Serialize;

use crate::capture::Capture;
use crate::extract::{Grid, chars_alike, mode_of, read_grid_sized, styles_alike};
use crate::glyphs::{FontModel, Style, alphabet};
use crate::spec::{Expect, Rgb, Side, Spec};

/// Largest per-channel foreground error accepted. Measured over the 20-scene
/// corpus (2,045 inked cells per editor): max 5 on GNU PGTK, 4 on romacs.
/// A declared colour 9 levels away from what was drawn fails
/// (`declared_colour_tolerance_boundary`).
pub const TOL_FG: u8 = 8;

/// Largest per-channel background error accepted. Measured: 0 on both
/// editors for every one of 5,600 cells; 2 leaves room for a lossless
/// colour-space round trip, not for a different colour.
pub const TOL_BG: u8 = 2;

/// Largest offset of any glyph from its declared cell position. Every one of
/// the 2,045 inked cells of the corpus measured (0, 0) on GNU and on romacs,
/// so placement is exact: one glyph one pixel off fails. This is measured
/// for Liberation Mono at 20 px; another font or size must be revalidated
/// with the corpus before trusting it.
pub const MAX_GLYPH_OFFSET: i32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Kind {
    Text,
    Colour,
    Style,
    Geometry,
    Cursor,
    Stray,
    Fringe,
}

#[derive(Clone, Debug, Serialize)]
pub struct Failure {
    pub kind: Kind,
    pub row: Option<usize>,
    pub col: Option<usize>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub scene: String,
    pub rows_read: Vec<String>,
    pub failures: Vec<Failure>,
    /// Places where the declaration was accepted only because the font draws
    /// the declared and the read character (or style) identically, so no
    /// capture can tell them apart.
    pub pixel_equivalent: Vec<String>,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    pub fn has(&self, kind: Kind) -> bool {
        self.failures.iter().any(|f| f.kind == kind)
    }

    fn fail(&mut self, kind: Kind, row: Option<usize>, col: Option<usize>, detail: String) {
        self.failures.push(Failure {
            kind,
            row,
            col,
            detail,
        });
    }
}

/// Declared attributes of one cell.
struct Want {
    ch: char,
    bg: Rgb,
    fg: Rgb,
    style: Style,
    underline: bool,
    cursor: bool,
}

fn want(spec: &Spec, exp: &Expect, row: usize, col: usize) -> Result<Want, String> {
    let (mut fg, mut bg) = (spec.frame.fg, spec.frame.bg);
    let (mut bold, mut italic, mut underline) = (false, false, false);
    let ch = exp
        .rows
        .get(row)
        .and_then(|r| r.text.chars().nth(col))
        .unwrap_or(' ');
    for run in exp.rows.get(row).map_or(&[][..], |r| &r.runs[..]) {
        if (run.start..run.end).contains(&col) {
            fg = run.fg.unwrap_or(fg);
            bg = run.bg.unwrap_or(bg);
            bold |= run.bold;
            italic |= run.italic;
            underline |= run.underline;
        }
    }
    if bold && italic {
        return Err(format!("row {row} col {col}: bold italic is not supported"));
    }
    let style = if bold {
        Style::Bold
    } else if italic {
        Style::Italic
    } else {
        Style::Regular
    };
    let cursor = exp.cursor.is_some_and(|c| c.row == row && c.col == col);
    // A filled box cursor paints the cell in the cursor colour and the glyph
    // in the cell's own background (GNU's cursor GC; romacs measured alike).
    let (bg, fg) = if cursor {
        (spec.cursor_color, bg)
    } else {
        (bg, fg)
    };
    Ok(Want {
        ch,
        bg,
        fg,
        style,
        underline,
        cursor,
    })
}

pub fn check(spec: &Spec, cap: &Capture) -> Result<Report, String> {
    let exp = spec
        .expect
        .as_ref()
        .ok_or_else(|| format!("{}: spec declares no `expect`", spec.name))?;
    let mut report = Report {
        scene: spec.name.clone(),
        rows_read: Vec::new(),
        failures: Vec::new(),
        pixel_equivalent: Vec::new(),
    };
    if (cap.width, cap.height) != (spec.frame.width, spec.frame.height) {
        report.fail(
            Kind::Geometry,
            None,
            None,
            format!(
                "capture is {}x{}, declared frame {}x{}",
                cap.width, cap.height, spec.frame.width, spec.frame.height
            ),
        );
        return Ok(report);
    }
    let declared: Vec<char> = exp.rows.iter().flat_map(|r| r.text.chars()).collect();
    let model = FontModel::load(&spec.font, &alphabet(declared))?;
    let (cw, lh) = (model.cell_w, model.line_h());
    let fringe = spec.frame.fringe as i32;
    let [ox, oy] = exp.origin;
    let (w, h) = (cap.width as i32, cap.height as i32);
    let cols = exp
        .cols
        .unwrap_or_else(|| ((w - fringe - ox) / cw).max(0) as usize);
    let rows = ((h - oy) / lh).max(0) as usize;
    if exp.rows.len() > rows {
        report.fail(
            Kind::Text,
            Some(rows),
            None,
            format!("{} rows declared, {rows} fit in the frame", exp.rows.len()),
        );
    }
    let grid = read_grid_sized(cap, &model, [ox, oy], rows, cols);
    report.rows_read = (0..rows).map(|r| grid.row_text(r)).collect();
    check_cells(spec, exp, &model, &grid, &mut report)?;
    let area = Area {
        origin: [ox, oy],
        rows,
        cols,
        cw,
        lh,
    };
    check_outside(spec, exp, cap, &area, &mut report);
    Ok(report)
}

fn check_cells(
    spec: &Spec,
    exp: &Expect,
    model: &FontModel,
    grid: &Grid,
    report: &mut Report,
) -> Result<(), String> {
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let want = want(spec, exp, r, c)?;
            let cell = grid.cell(r, c);
            let (row, col) = (Some(r), Some(c));
            // Text.
            let text_ok = chars_alike(model, cell.ch, want.ch);
            if !text_ok {
                let runner = cell
                    .runner_up
                    .map(|(ch, s)| format!(", runner-up {ch:?} {s:.3}"))
                    .unwrap_or_default();
                report.fail(
                    Kind::Text,
                    row,
                    col,
                    format!(
                        "declared {:?}, read {:?} (score {:.3}{runner})",
                        want.ch, cell.ch, cell.score
                    ),
                );
            } else if cell.ch != want.ch {
                report.pixel_equivalent.push(format!(
                    "row {r} col {c}: declared {:?}, drawn alike {:?}",
                    want.ch, cell.ch
                ));
            }
            // Cursor and background: a cursor missing from its cell, or a
            // cursor-coloured cell anywhere else, is a cursor failure.
            if cell.bg.dist(want.bg) > TOL_BG {
                let kind = if want.cursor || cell.bg.dist(spec.cursor_color) <= TOL_BG {
                    Kind::Cursor
                } else {
                    Kind::Colour
                };
                report.fail(
                    kind,
                    row,
                    col,
                    format!("background {}, declared {}", cell.bg, want.bg),
                );
            }
            // Underline is asserted on every cell, blank or not.
            if cell.underline != want.underline {
                report.fail(
                    Kind::Style,
                    row,
                    col,
                    format!("underline {}, declared {}", cell.underline, want.underline),
                );
            }
            if want.ch == ' ' || cell.ch == ' ' || !text_ok {
                continue;
            }
            // Foreground.
            if let Some(fg) = cell.fg
                && fg.dist(want.fg) > TOL_FG
            {
                report.fail(
                    Kind::Colour,
                    row,
                    col,
                    format!("foreground {fg} on {:?}, declared {}", cell.ch, want.fg),
                );
            }
            // Weight and slant.
            if let Some(style) = cell.style
                && style != want.style
            {
                if styles_alike(model, want.ch, style, want.style) {
                    report.pixel_equivalent.push(format!(
                        "row {r} col {c}: {:?} is drawn alike in {style:?} and {:?}",
                        want.ch, want.style
                    ));
                } else {
                    report.fail(
                        Kind::Style,
                        row,
                        col,
                        format!("{:?} read {style:?}, declared {:?}", cell.ch, want.style),
                    );
                }
            }
            // Geometry.
            if cell.dx.abs() > MAX_GLYPH_OFFSET || cell.dy.abs() > MAX_GLYPH_OFFSET {
                report.fail(
                    Kind::Geometry,
                    row,
                    col,
                    format!("{:?} drawn at offset ({}, {})", cell.ch, cell.dx, cell.dy),
                );
            }
        }
    }
    Ok(())
}

struct Area {
    origin: [i32; 2],
    rows: usize,
    cols: usize,
    cw: i32,
    lh: i32,
}

/// Pixels outside the text grid: the fringes, and any remainder of the
/// frame that whole cells do not cover.
fn check_outside(spec: &Spec, exp: &Expect, cap: &Capture, a: &Area, report: &mut Report) {
    let (w, h) = (cap.width as i32, cap.height as i32);
    let fringe = spec.frame.fringe as i32;
    let [ox, oy] = a.origin;
    let (gx1, gy1) = (ox + a.cols as i32 * a.cw, oy + a.rows as i32 * a.lh);
    let in_fringe = |x: i32| fringe > 0 && (x < fringe || x >= w - fringe);
    let mut stray = 0usize;
    let mut first = None;
    for y in 0..h {
        for x in 0..w {
            let in_grid = x >= ox && x < gx1 && y >= oy && y < gy1;
            if in_grid || in_fringe(x) {
                continue;
            }
            let p = cap.get(x, y).unwrap_or(spec.frame.bg);
            if p.dist(spec.frame.bg) > TOL_BG {
                stray += 1;
                first.get_or_insert((x, y, p));
            }
        }
    }
    if let Some((x, y, p)) = first {
        report.fail(
            Kind::Stray,
            None,
            None,
            format!(
                "{stray} pixels outside the text grid differ from the frame background; \
                 first {p} at ({x}, {y})"
            ),
        );
    }
    if fringe == 0 {
        return;
    }
    let fbg = spec.frame.fringe_bg.unwrap_or(spec.frame.bg);
    let ffg = spec.frame.fringe_fg.unwrap_or(spec.frame.fg);
    for side in [Side::Left, Side::Right] {
        let xs = match side {
            Side::Left => 0..fringe,
            Side::Right => w - fringe..w,
        };
        let band = |y0: i32, y1: i32| -> Vec<Rgb> {
            (y0..y1)
                .flat_map(|y| xs.clone().map(move |x| (x, y)))
                .filter_map(|(x, y)| cap.get(x, y))
                .collect()
        };
        // Bitmaps are found against the background actually drawn, so that a
        // wrong fringe background is one failure, not one per row.
        let bg = mode_of(&band(0, h));
        if bg.dist(fbg) > TOL_BG {
            report.fail(
                Kind::Fringe,
                None,
                None,
                format!("{side:?} fringe background {bg}, declared {fbg}"),
            );
        }
        for r in 0..=a.rows {
            let (y0, y1) = if r < a.rows {
                (oy + r as i32 * a.lh, oy + (r as i32 + 1) * a.lh)
            } else {
                (gy1, h)
            };
            let px = band(y0, y1);
            let ink = px.iter().filter(|p| p.dist(bg) > TOL_BG).count();
            let marked = exp
                .fringe_marks
                .iter()
                .any(|m| m.row == r && m.side == side);
            if marked && !px.iter().any(|p| p.dist(ffg) <= TOL_FG) {
                report.fail(
                    Kind::Fringe,
                    Some(r),
                    None,
                    format!("{side:?} fringe: declared bitmap in {ffg} not drawn"),
                );
            }
            if !marked && ink > 0 {
                report.fail(
                    Kind::Fringe,
                    Some(r),
                    None,
                    format!("{side:?} fringe: {ink} pixels drawn where no bitmap is declared"),
                );
            }
        }
    }
}
