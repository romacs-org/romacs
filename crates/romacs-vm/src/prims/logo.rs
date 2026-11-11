//! `romacs--logo-frame`: a frame of the Romacs mark and wordmark in braille
//! cells (the romacs-logo crate) for lisp/romacs/romacs-landing.el.
use super::leaf_scalars::number;
use super::orderless::list;
use super::*;

// The logo's frame, COLS by ROWS cells joined by newlines, for cells
// CELL-ASPECT as wide as high, TIME seconds into its motion (0 when
// nil): each lit cell's `face' is (:foreground "#RRGGBB").  Signals
// `args-out-of-range' outside 1..400 columns, 1..200 rows or a cell
// aspect in (0, 2].
defsubr! { name = "romacs--logo-frame", gnu = "romacs:logo", effect = Reads,
fn romacs_logo_frame(cx, cols: Value, rows: Value, cell_aspect: Value; optional time: Value) {
    let count = |v: Value| v.integer_value().map(|n| usize::try_from(n).unwrap_or(0)).ok_or(Fault::Wrong("integerp", v));
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
