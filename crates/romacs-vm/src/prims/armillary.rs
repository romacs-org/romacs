//! `romacs--armillary-frame`: a frame of the ASCII armillary sphere
//! (the romacs-armillary crate) for rlisp/romacs-armillary.el.
use super::leaf_scalars::number;
use super::*;
use romacs_armillary::{cells, shade_runs, text, View};

// The sphere's frame, COLS by ROWS cells joined by newlines, turned by
// PITCH, YAW and ROLL, at ZOOM, for cells CELL-ASPECT wide per unit of
// height, with the star field when STARS.  With FACES, a vector of faces
// dark to light, each lit cell's `face' is the one its brightness falls
// in, a star's `+' the last.  Signals `args-out-of-range' outside
// 1..200 columns, 1..100 rows or a cell aspect in (0, 2].
defsubr! { name = "romacs--armillary-frame", gnu = "romacs:armillary", effect = Reads,
fn romacs_armillary_frame(cx, cols: Value, rows: Value, pitch: Value, yaw: Value, roll: Value, zoom: Value, cell_aspect: Value, stars: Value; optional faces: Value) {
    let count = |v: Value| v.integer_value().map(|n| usize::try_from(n).unwrap_or(0)).ok_or(Fault::Wrong("integerp", v));
    let view = View {
        cols: count(cols)?, rows: count(rows)?,
        pitch: number(cx, pitch)?, yaw: number(cx, yaw)?, roll: number(cx, roll)?,
        zoom: number(cx, zoom)?, cell_aspect: number(cx, cell_aspect)?, stars: stars != Value::NIL,
    };
    if !view.valid() { return Err(Fault::Range(cols, rows)); }
    let faces = if faces == Value::NIL { vec![] } else { cx.vector_values(faces)? };
    let cells = cells(&view);
    let runs = shade_runs(&cells, view.cols, faces.len()).into_iter().map(|(range, i)| (range, faces[i])).collect::<Vec<_>>();
    let text = text(&cells, view.cols).into_bytes();
    Ok(cx.faced_ascii(text, &runs))
}}
