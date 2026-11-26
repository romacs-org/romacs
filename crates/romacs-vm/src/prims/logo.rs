//! Frames the romacs-logo crate draws, for Lisp's still frames:
//! `romacs--logo-frame`, the Romacs mark and wordmark in braille cells
//! (lisp/ui/romacs-landing.el), and `romacs--banner-frame`, a banner
//! (lisp/ui/romacs-banner.el).
use super::leaf_scalars::number;
use super::orderless::{elements, list};
use super::*;

// The logo's frame, COLS by ROWS cells joined by newlines, for cells
// CELL-ASPECT as wide as high, TIME seconds into its motion (0 when
// nil): each lit cell's `face' is (:foreground "#RRGGBB").  Signals
// `args-out-of-range' outside 1..400 columns, 1..200 rows or a cell
// aspect in (0, 2].
defsubr! { name = "romacs--logo-frame", gnu = "romacs:logo", effect = Reads,
fn romacs_logo_frame(cx, cols: Value, rows: Value, cell_aspect: Value; optional time: Value) {
    let count = |v: Value| v.integer_value().map(|n| usize::try_from(n).unwrap_or(0)).ok_or_else(|| Fault::Wrong("integerp", v));
    let view = romacs_logo::View { cols: count(cols)?, rows: count(rows)?, cell_aspect: number(cx, cell_aspect)? };
    if !view.valid() { return Err(Fault::Range(cols, rows)); }
    let time = if time == Value::NIL { 0.0 } else { number(cx, time)? };
    let cells = romacs_logo::frame(view, time);
    let text = romacs_logo::text(&cells, view.cols);
    // Runs of one colour, in byte offsets of TEXT.
    let mut runs: Vec<(std::ops::Range<usize>, u32)> = vec![];
    let mut at = 0;
    for (i, &(c, colour)) in cells.iter().enumerate() {
        if i > 0 && i % view.cols == 0 { at += 1; }
        let end = at + c.len_utf8();
        if let Some(colour) = colour {
            match runs.last_mut() {
                Some((range, last)) if *last == colour && range.end == at => range.end = end,
                _ => runs.push((at..end, colour)),
            }
        }
        at = end;
    }
    let foreground = cx.intern(":foreground");
    let mut faces: Vec<(u32, Value)> = vec![];
    let mut faced = Vec::with_capacity(runs.len());
    for (range, colour) in runs {
        let face = match faces.iter().find(|(c, _)| *c == colour) {
            Some(&(_, face)) => face,
            None => {
                let hex = cx.string(&format!("#{colour:06x}"));
                let face = list(cx, &[foreground, hex]);
                faces.push((colour, face));
                face
            }
        };
        faced.push((range, face));
    }
    Ok(cx.faced_text(text, &faced))
}}

// A banner's frame (romacs_logo::banner), COLS by ROWS cells joined by
// newlines, in PATTERN (`wave', `rain', `sparkle' or `sweep'), seeded by
// EPOCH (0 when nil), TIME seconds into its motion (0 when nil): each lit
// cell's `face' is the face of FACES its colour stop names.  The display
// draws the same frames, so this is its still frame.  Signals
// `args-out-of-range' outside 1..400 columns, 1..200 rows or 1..4 faces.
defsubr! { name = "romacs--banner-frame", gnu = "romacs:logo", effect = Reads,
fn romacs_banner_frame(cx, pattern: Value, cols: Value, rows: Value, faces: Value; optional epoch: Value, time: Value) {
    use romacs_logo::banner;
    let count = |v: Value| v.integer_value().map(|n| usize::try_from(n).unwrap_or(0)).ok_or_else(|| Fault::Wrong("integerp", v));
    let shape = banner::Pattern::named(&cx.symbol_name(pattern)?).ok_or(Fault::Wrong("romacs-banner-pattern-p", pattern))?;
    let faces = elements(cx, faces)?;
    let (cols_v, rows_v) = (cols, rows);
    let (cols, rows) = (count(cols)?, count(rows)?);
    if !banner::valid(cols, rows, faces.len()) { return Err(Fault::Range(cols_v, rows_v)); }
    let epoch = if epoch == Value::NIL { 0 } else { epoch.integer_value().and_then(|n| u64::try_from(n).ok()).ok_or(Fault::Wrong("natnump", epoch))? };
    let time = if time == Value::NIL { 0.0 } else { number(cx, time)? };
    let cells = banner::frame(shape, cols, rows, faces.len(), epoch, time);
    let mut text = String::with_capacity(cells.len() * 3 + rows);
    // Runs of one stop, in byte offsets of TEXT.
    let mut runs: Vec<(std::ops::Range<usize>, Value)> = vec![];
    for (i, &(c, stop)) in cells.iter().enumerate() {
        if i > 0 && i % cols == 0 { text.push('\n'); }
        let start = text.len();
        text.push(c);
        if let Some(face) = stop.map(|s| faces[s]) {
            match runs.last_mut() {
                Some((range, last)) if *last == face && range.end == start => range.end = text.len(),
                _ => runs.push((start..text.len(), face)),
            }
        }
    }
    Ok(cx.faced_text(text, &runs))
}}
