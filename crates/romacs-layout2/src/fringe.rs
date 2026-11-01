//! GNU's fringes beside a window's text area (fringe.c): their background
//! in the `fringe' face, the vertical border at the left edge of a window
//! with another left of it (gui_draw_vertical_border), and the bitmap
//! update_window_fringes chooses for each row.
use romacs_display_protocol::frame_glyphs::{FringeBitmapData, FringeSide, GlyphRowRole};
use romacs_display_protocol::{Color, DisplayWindowId, FaceId, FrameGlyphBuffer, Rect};
use romacs_layout_input::{BitmapAlign, FringeBitmap, Indicators, Row, WindowChrome, WindowSnapshot};

/// How a laid-out row breaks: it CONTINUED onto the next row, it is a
/// CONTINUATION of the row above, or the text area TRUNCATED it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RowMarks {
    pub continued: bool,
    pub continuation: bool,
    pub truncated: bool,
}

/// Where a window's text area lies in the frame: its origin OX, OY, its
/// height, the frame's line height, the
/// length of the window's text, whether its last row holds the text's end,
/// and the first face id of the window's face table.
pub(crate) struct Area {
    pub ox: f32,
    pub oy: f32,
    pub body_height: f32,
    pub line: f32,
    pub text_len: usize,
    pub reached_end: bool,
    pub face_base: u32,
}

fn sides(indicators: &Indicators) -> [&[Option<FringeBitmap>; 2]; 8] {
    [&indicators.truncation, &indicators.continuation, &indicators.top, &indicators.bottom,
     &indicators.top_bottom, &indicators.up, &indicators.down, &indicators.empty_line]
}

/// The faces of the bitmaps CHROME's fringes may show, as indexes into the
/// window's face table.
pub(crate) fn bitmap_faces(chrome: &WindowChrome) -> impl Iterator<Item = usize> + '_ {
    sides(&chrome.indicators).into_iter().flatten().flatten().map(|bitmap| bitmap.face.0 as usize)
}

/// Paint SNAPSHOT's fringes beside its text area at AREA, the border at
/// its left edge, and each of ROWS' indicators as MARKS and the window's
/// text say.
pub(crate) fn paint(frame: &mut FrameGlyphBuffer, snapshot: &WindowSnapshot, rows: &[Row], marks: &[RowMarks], area: Area) -> Result<(), String> {
    let chrome = &snapshot.chrome;
    let (left, _) = chrome.insets();
    let outer_left = area.ox - left as f32;
    let width = snapshot.width as f32;
    let fringe_x = [
        area.ox - chrome.left_fringe as f32 - if chrome.outside_margins { chrome.left_margin as f32 } else { 0.0 },
        area.ox + width + if chrome.outside_margins { chrome.right_margin as f32 } else { 0.0 },
    ];
    let fringe_width = [chrome.left_fringe as f32, chrome.right_fringe as f32];
    let window = DisplayWindowId::new(i64::try_from(snapshot.id).map_err(|e| e.to_string())?);
    // The text area's rows, from a pixel left of the window for the border.
    let (_, right) = chrome.insets();
    frame.set_draw_context(window, GlyphRowRole::Text,
        Some(Rect::new(outer_left - 1.0, area.oy, left as f32 + width + right as f32 + 1.0, area.body_height)));
    let face = snapshot.faces.get(chrome.fringe_face.0 as usize).ok_or("missing fringe face")?;
    for side in 0..2 {
        if fringe_width[side] > 0.0 {
            frame.add_background(fringe_x[side], area.oy, fringe_width[side], area.body_height, rgb(face.background));
        }
    }
    // pgtk_draw_vertical_window_border: a pixel wide, at the window's left
    // edge, or the pixel before it without a left fringe, down the rows.
    if let Some(border) = chrome.border.and_then(|b| snapshot.faces.get(b.0 as usize)) {
        let x = outer_left - if chrome.left_fringe == 0 { 1.0 } else { 0.0 };
        frame.add_border(x, area.oy, 1.0, (area.body_height - 1.0).max(0.0), rgb(border.foreground));
    }
    if chrome.left_fringe + chrome.right_fringe > 0 {
        for (row, bitmaps) in row_bitmaps(chrome, rows, marks, &area) {
            for (side, bitmap) in bitmaps.into_iter().enumerate() {
                let Some(bitmap) = bitmap else { continue };
                frame.register_fringe_bitmap(bitmap.number, FringeBitmapData {
                    bits: bitmap.bits.clone(),
                    width: bitmap.width,
                    height: u8::try_from(bitmap.bits.len()).unwrap_or(u8::MAX),
                    period: bitmap.period,
                    align: match bitmap.align { BitmapAlign::Center => 0, BitmapAlign::Top => 1, BitmapAlign::Bottom => 2 },
                });
                frame.push_fringe_bitmap(fringe_x[side], area.oy + row.y, fringe_width[side], row.height, bitmap.number,
                    FaceId::new(area.face_base + bitmap.face.0), if side == 0 { FringeSide::Left } else { FringeSide::Right });
            }
        }
    }
    frame.set_draw_context(window, GlyphRowRole::Text, Some(Rect::new(area.ox, area.oy, width, area.body_height)));
    Ok(())
}

/// update_window_fringes's choice for each visible row of ROWS and each
/// empty row below the text, [left, right], in its order: truncation, the
/// buffer's first and last rows, continuation, empty lines, then the
/// arrows saying the text goes on above or below the window.
fn row_bitmaps<'a>(chrome: &'a WindowChrome, rows: &[Row], marks: &[RowMarks], area: &Area) -> Vec<(Row, [Option<&'a FringeBitmap>; 2])> {
    let ind = &chrome.indicators;
    let visible = |row: &Row| row.y < area.body_height && row.y + row.height > 0.0;
    let whole = |row: &Row| row.y >= 0.0 && row.y + row.height <= area.body_height;
    let end_of = |i: usize| rows.get(i + 1).map_or(if area.reached_end { area.text_len } else { 0 }, |r| r.start);
    // The top row marks the buffer's start, else says text is above; the
    // first row reaching its end, else the bottom row, says the like below.
    let top = rows.iter().position(visible);
    let bob = top.filter(|&i| ind.bob && rows[i].start == 0 && rows[i].y >= 0.0);
    let eob = rows.iter().enumerate().position(|(i, row)| visible(row) && ind.eob && area.reached_end && end_of(i) >= area.text_len && whole(row));
    let bottom = if eob.is_some() { None } else { rows.iter().position(|row| visible(row) && row.y + row.height >= area.body_height) };
    let mut out = vec![];
    for (i, (row, mark)) in rows.iter().zip(marks).enumerate().filter(|(_, (row, _))| visible(row)) {
        let pick = |side: usize| -> Option<&'a FringeBitmap> {
            let truncated = side == 1 && mark.truncated;
            let broken = if side == 0 { mark.continuation } else { mark.continued };
            let at_top = bob == Some(i) && ind.top[side].is_some();
            let at_bottom = eob == Some(i) && ind.bottom[side].is_some();
            if truncated { ind.truncation[side].as_ref() }
            else if at_top && at_bottom { ind.top_bottom[side].as_ref().or(ind.top[side].as_ref()) }
            else if at_top { ind.top[side].as_ref() }
            else if at_bottom { ind.bottom[side].as_ref() }
            else if broken && ind.continuation[side].is_some() { ind.continuation[side].as_ref() }
            else if top == Some(i) && bob.is_none() && ind.up[side].is_some() { ind.up[side].as_ref() }
            else if bottom == Some(i) && ind.down[side].is_some() { ind.down[side].as_ref() }
            else { None }
        };
        out.push((*row, [pick(0), pick(1)]));
    }
    // Rows past the end of the text show `indicate-empty-lines'.
    if area.reached_end && ind.empty_line.iter().any(Option::is_some) && area.line > 0.0 {
        let mut y = rows.last().map_or(0.0, |row| row.y + row.height);
        while y < area.body_height {
            out.push((Row { start: area.text_len, y, height: area.line }, [ind.empty_line[0].as_ref(), ind.empty_line[1].as_ref()]));
            y += area.line;
        }
    }
    out
}

fn rgb(c: [u8; 3]) -> Color {
    Color::from_pixel((u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]))
}
