//! A window's view of its lines' rows: which rows show from its start, at
//! what y, where its text ends, where point is; and, for a paint, their
//! glyphs in the frame with the fringes, the line numbers, the cursor and
//! the bands around them. The rows come from the line cache; what depends
//! on the window (its start, its scroll, its height, point) is worked out
//! here on each layout, a row at a time.
use crate::faces::{self, Shows, Table};
use crate::fringe::{self, RowMarks};
use crate::line::{LineText, LineView, Lines};
use crate::rows::{Cache, drawn};
use crate::faces::Faces;
use crate::wrap::{Ask, Clip, Context, Fill, Ink, Mode, Op, Pointed, Restart, Row as Wrapped, Started, Walk, Walked, settled_cursor};
use crate::{CursorRect, Metrics, px};
use romacs_display_protocol::frame_glyphs::GlyphRowRole;
use romacs_display_protocol::{Color, DisplayWindowId, FaceId, FrameGlyph, FrameGlyphBuffer, Rect};
use std::ops::Not;
use romacs_font::FontResolver;
use romacs_layout_input::{FaceHandle, LineWrap, PointPlace, Row, TextSlice, WindowEnd, WindowSnapshot};

/// What a layout works with beside its snapshot: the lines kept, the
/// faces realised, and the fonts.
pub(crate) struct Kept<'a> {
    pub(crate) cache: &'a mut Cache,
    pub(crate) faces: &'a mut Faces,
    pub(crate) resolver: &'a mut FontResolver,
}

/// One line of a window's text in a pass: its entry's key, its bytes in
/// the text, and where its slots' table indexes are kept.
#[derive(Clone, Copy)]
struct Seen {
    key: u64,
    start: usize,
    end: usize,
    newline: bool,
    slots: (usize, usize),
}

/// A window's lines as a pass reads them, each once.
struct Pass<'a> {
    snapshot: &'a WindowSnapshot,
    table: Table<'a>,
    view: LineView,
    /// The lines from the window start's line on, as far as read.
    lines: Lines<'a>,
    seen: Vec<Seen>,
    /// Each seen line's slots' table indexes, end to end.
    slots: Vec<u32>,
    /// What every face of the table shows in the window's whole text and
    /// its bands, once something asked.
    shown: Option<Vec<Shows>>,
    /// The first line's start.
    from: usize,
}

impl<'a> Pass<'a> {
    fn new(snapshot: &'a WindowSnapshot, from: usize) -> Self {
        Pass { snapshot, table: Table::new(&snapshot.faces), view: LineView::default(), lines: Lines::from(snapshot, from), seen: vec![], slots: vec![], shown: None, from }
    }

    /// Line N from the window start's line, read now if not yet.
    fn line(&mut self, n: usize, kept: &mut Kept) -> Result<Option<Seen>, String> {
        while self.seen.len() <= n {
            if !self.lines.next(&mut self.view, &mut self.table, kept.faces, kept.resolver)? { return Ok(None); }
            let seen = see(&self.view, self.snapshot, &mut self.slots, kept);
            self.seen.push(seen);
        }
        Ok(Some(self.seen[n]))
    }

    fn slots(&self, seen: &Seen) -> &[u32] { &self.slots[seen.slots.0..seen.slots.1] }

    /// The faces the window keeps whatever its text shows: a band's own
    /// face sets its height even where no character uses it, as the faces
    /// of the fringes' bitmaps set their colours, and an extend face
    /// paints with no character.
    fn keeps(&self, index: usize) -> bool {
        let snapshot = self.snapshot;
        let bands = [(&snapshot.header_line, snapshot.header_line_face), (&snapshot.mode_line, snapshot.mode_line_face)];
        bands.iter().any(|(band, face)| band.is_some() && face.0 as usize == index)
            || fringe::bitmap_faces(&snapshot.chrome).any(|face| face == index)
            || snapshot.text.extends.iter().any(|e| e.face.0 as usize == index)
    }

    /// What each face of the table shows in the window: `face_text`, from
    /// the lines' own sets.
    fn shown(&mut self, kept: &mut Kept) -> Result<&[Shows], String> {
        if self.shown.is_none() {
            let snapshot = self.snapshot;
            let mut shown = vec![Shows::default(); snapshot.faces.len()];
            // The continuation and truncation glyphs, in the default face.
            shown[0].add_text("\\$");
            // Every line: those above the window start's, then the rest.
            let mut above = Lines::from(snapshot, 0);
            let mut slots = vec![];
            while above_start(&mut above, &mut self.view, self.from, &mut self.table, kept)? {
                slots.clear();
                let seen = see(&self.view, snapshot, &mut slots, kept);
                merge(&mut shown, &kept.cache.line(seen.key), &slots);
            }
            let mut n = 0;
            while let Some(seen) = self.line(n, kept)? {
                merge(&mut shown, &kept.cache.line(seen.key), &self.slots[seen.slots.0..seen.slots.1]);
                n += 1;
            }
            for (band, face) in [(&snapshot.header_line, snapshot.header_line_face), (&snapshot.mode_line, snapshot.mode_line_face)] {
                if let Some(band) = band { crate::line::slice_shows(band, face.0 as usize, &mut shown); }
            }
            if let Some(numbers) = &snapshot.line_numbers {
                for face in [numbers.face, numbers.current] { shown[face.0 as usize].add_text("0123456789 "); }
            }
            // A display table's truncation and continuation glyphs show in
            // the default face, from whichever font has them.
            for glyph in snapshot.chrome.line_glyphs.into_iter().flatten() { shown[0].add_char(glyph); }
            self.shown = Some(shown);
        }
        Ok(self.shown.as_deref().unwrap_or_default())
    }

    /// The metrics of the face at INDEX as the window realises it: zeros
    /// for a face it shows no character in and does not keep.
    fn metrics(&mut self, index: usize, kept: &mut Kept) -> Result<Metrics, String> {
        Ok(self.table.get(index, kept.faces, kept.resolver)?.metrics)
    }
}

/// Read the next line of LINES into VIEW when it starts before FROM.
fn above_start(lines: &mut Lines, view: &mut LineView, from: usize, table: &mut Table, kept: &mut Kept) -> Result<bool, String> {
    if lines.at() >= from { return Ok(false); }
    lines.next(view, table, kept.faces, kept.resolver)
}

/// VIEW's line as a pass keeps it, its slots' indexes added to SLOTS.
fn see(view: &LineView, snapshot: &WindowSnapshot, slots: &mut Vec<u32>, kept: &mut Kept) -> Seen {
    let text = &snapshot.text.utf8[view.start..view.end];
    let key = kept.cache.key(view, text, snapshot.id);
    let from = slots.len();
    slots.extend_from_slice(&view.indexes);
    Seen { key, start: view.start, end: view.end, newline: view.newline.is_some(), slots: (from, slots.len()) }
}

/// Add what LINE shows to SHOWN, by the table index SLOTS gives each slot.
fn merge(shown: &mut [Shows], line: &LineText, slots: &[u32]) {
    for (shows, &index) in line.shows.iter().zip(slots) {
        if shows.used { shown[index as usize].add(shows); }
    }
}

/// A row as a window shows it: the line it is of, the wrapped row, its
/// top, and the scroll and gutter of its line.
struct Placed {
    seen: Seen,
    row: Wrapped,
    y: f32,
    shift: f32,
    x0: f32,
    /// It shows its line's left truncation glyph.
    left_mark: bool,
}

/// What a window's rows come to: the journal, each row's marks, the rows
/// to paint, the cursor's rectangle, and whether the text's end was reached.
struct View {
    end: WindowEnd,
    marks: Vec<RowMarks>,
    placed: Vec<Placed>,
    cursor: Option<CursorRect>,
    reached_end: bool,
}

/// The window's own inputs, read once.
struct Frame {
    body_height: f32,
    truncate: bool,
    word: bool,
    special: char,
    left_glyph: char,
    glyphs_mark: bool,
    last_x: f32,
    dollar: f32,
    point_line: usize,
}

/// SNAPSHOT's checks before anything is laid out; true for a window
/// whose body has no width, which shows nothing.
pub(crate) fn validate(snapshot: &WindowSnapshot) -> Result<bool, String> {
    let text = &snapshot.text.utf8;
    if snapshot.height == 0
        || snapshot.faces.is_empty()
        || !text.is_char_boundary(snapshot.start_byte)
        || !text.is_char_boundary(snapshot.point_byte)
    {
        return Err("invalid snapshot dimensions, positions, face table or presentation".into());
    }
    if snapshot.width == 0 { return Ok(true); }
    crate::validate_runs(&snapshot.text, snapshot.faces.len())?;
    if snapshot.stretches.windows(2).any(|w| w[0].byte >= w[1].byte)
        || snapshot.stretches.iter().any(|s| text.get(s.byte..s.byte + 1) != Some(" ")) {
        return Err("stretches out of order or not on a placeholder space".into());
    }
    if snapshot.images.windows(2).any(|w| w[0].byte >= w[1].byte)
        || snapshot.images.iter().any(|i| text.get(i.byte..i.byte + 1) != Some(" ")) {
        return Err("images out of order or not on a placeholder space".into());
    }
    for (band, face) in [(&snapshot.header_line, snapshot.header_line_face), (&snapshot.mode_line, snapshot.mode_line_face)] {
        let Some(band) = band else { continue };
        crate::validate_runs(band, snapshot.faces.len())?;
        if face.0 as usize >= snapshot.faces.len() || band.utf8.chars().any(char::is_control) {
            return Err("invalid mode-line face or control character".into());
        }
    }
    if let Some(numbers) = &snapshot.line_numbers
        && (numbers.face.0 as usize >= snapshot.faces.len() || numbers.current.0 as usize >= snapshot.faces.len()) {
        return Err("invalid line-number face".into());
    }
    if snapshot.text.extends.iter().any(|e| e.face.0 as usize >= snapshot.faces.len())
        || snapshot.text.extends.windows(2).any(|w| w[0].byte >= w[1].byte) {
        return Err("invalid extend face".into());
    }
    Ok(false)
}

/// Line numbers take each row's first glyphs, its line's label or as many
/// blanks, in cells of their face: the gutter's width at BYTE, the label
/// when a row starting there starts its line, whether it is point's line.
fn gutter(snapshot: &WindowSnapshot, cell: f32, byte: usize) -> (f32, Option<&str>, bool) {
    let Some(numbers) = &snapshot.line_numbers else { return (0.0, None, false) };
    let at = numbers.labels.partition_point(|(b, ..)| *b <= byte);
    match at.checked_sub(1).map(|i| &numbers.labels[i]) {
        Some((b, label, current)) if *b == byte => (px::count(label.len()) * cell, Some(label.as_str()), *current),
        Some((_, label, current)) => (px::count(label.len()) * cell, None, *current),
        None => (px::of(numbers.columns) * cell, None, false),
    }
}

/// What a layout of a window does beside reporting its rows: nothing, a
/// paint of it, or only its cursor, into a frame that shows it already.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Paint { No, Whole, Cursor }

/// Lay SNAPSHOT's window out and report its rows (`WindowEnd`); with a
/// FRAME, paint it there at OX, OY, or as PAINT says only its cursor, with
/// CURSOR where it owns one.
pub(crate) fn append_window(snapshot: &WindowSnapshot, mut frame: Option<&mut FrameGlyphBuffer>, kept: &mut Kept, ox: f32, oy: f32,
                            cursor: Option<(bool, Color)>, paint: Paint) -> Result<WindowEnd, String> {
    // A body width of zero is legal: an empty body paints nothing and
    // displays nothing, which makes window-end equal window-start.
    if validate(snapshot)? {
        return Ok(WindowEnd { id: snapshot.id, end: snapshot.start_byte, visible_end: snapshot.start_byte, body_height: 0.0, rows: vec![], point: None });
    }
    let text = &snapshot.text.utf8;
    let line_start = text[..snapshot.start_byte].rfind('\n').map_or(0, |at| at + 1);
    let mut pass = Pass::new(snapshot, line_start);
    // Above every face id an earlier window took, its band's copies included.
    let face_base = frame.as_deref().map_or(0, |frame| frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(0));
    // A paint publishes the window's faces first: its glyphs name them.
    let published = match frame.as_deref_mut().filter(|_| paint == Paint::Whole) {
        Some(frame) => {
            let shown = pass.shown(kept)?.to_vec();
            let keep: Vec<bool> = (0..snapshot.faces.len()).map(|index| pass.keeps(index)).collect();
            Some(faces::publish(&mut pass.table, &shown, |index| keep[index], frame, face_base, snapshot.underline_rules, kept.faces, kept.resolver)?)
        }
        None => None,
    };
    let default = pass.table.get(0, kept.faces, kept.resolver)?;
    let metrics0 = default.metrics;
    if face_base == 0 && paint == Paint::Whole && let Some(frame) = frame.as_deref_mut() { frame.char_height = metrics0.1; frame.char_width = metrics0.2; }
    // A band is as tall as its tallest face.
    let mut band_height = |band: &Option<TextSlice>, face: FaceHandle| -> Result<f32, String> {
        let Some(band) = band else { return Ok(0.0) };
        let mut height = 0.0f32;
        for index in std::iter::once(face.0 as usize).chain(band.face_runs.iter().map(|run| run.face.0 as usize)) {
            height = height.max(window_metrics(&mut pass, index, published.as_ref().map(|p| &p.0[..]), kept)?.1);
        }
        Ok(height)
    };
    let header_height = band_height(&snapshot.header_line, snapshot.header_line_face)?;
    let mode_height = band_height(&snapshot.mode_line, snapshot.mode_line_face)?;
    let body_height = (px::of(snapshot.height) - header_height - mode_height).max(0.0);
    // The text area starts below the header line: OY is its origin from here.
    let (window_top, oy) = (oy, oy + header_height);
    let window = DisplayWindowId::new(i64::try_from(snapshot.id).map_err(|e| e.to_string())?);
    if let Some(frame) = frame.as_deref_mut().filter(|_| paint == Paint::Whole) {
        frame.set_draw_context(window, GlyphRowRole::Text, Some(Rect::new(ox, oy, px::of(snapshot.width), body_height)));
    }
    // GNU: a row ends where its next glyph would pass the text area, less
    // the continuation (`\\`) or truncation (`$`) glyph it shows there on
    // a window without a right fringe, whose indicators show the break
    // instead (display_line, move_it_in_display_line_to, init_iterator).
    // A horizontally scrolled window truncates its lines (init_iterator).
    let truncate = snapshot.wrap == LineWrap::Truncate || !snapshot.hscroll.is_zero();
    // A display table's glyph, where it sets one, for GNU's `$` and `\\`.
    let [truncation, continuation] = snapshot.chrome.line_glyphs;
    let special = if truncate { truncation.unwrap_or('$') } else { continuation.unwrap_or('\\') };
    let glyphs_mark = snapshot.chrome.right_fringe == 0;
    let last_x = px::of(snapshot.width) - if glyphs_mark { default.advance(special, kept.faces, kept.resolver) } else { 0.0 };
    // A scrolled row's left truncation glyph, without a left fringe
    // (insert_left_trunc_glyphs): it hides the glyphs it covers.
    let left_glyph = truncation.unwrap_or('$');
    let dollar = if snapshot.chrome.left_fringe == 0 { default.advance(left_glyph, kept.faces, kept.resolver) } else { 0.0 };
    // Each line shows from its first visible x (GNU's first_visible_x):
    // point's line from the hscroll of point's row.
    let point_line = text[..snapshot.point_byte].rfind('\n').map_or(0, |at| at + 1);
    let window_frame = Frame { body_height, truncate, word: snapshot.wrap == LineWrap::Word && !truncate, special, left_glyph, glyphs_mark, last_x, dollar, point_line };
    let numbers_cell = match &snapshot.line_numbers {
        Some(numbers) => window_metrics(&mut pass, numbers.face.0 as usize, published.as_ref().map(|p| &p.0[..]), kept)?.2,
        None => 0.0,
    };
    let view = rows(snapshot, &window_frame, &mut pass, numbers_cell, metrics0, paint == Paint::Whole, kept)?;
    let Some(frame) = frame else { return Ok(view.end) };
    if paint == Paint::Cursor {
        add_cursor(frame, snapshot, (ox, oy, body_height), cursor, view.cursor);
        return Ok(view.end);
    }
    let (metrics, glyphless) = published.expect("a paint published its faces");
    let ids = Ids { base: face_base, len: u32::try_from(snapshot.faces.len()).map_err(|e| e.to_string())? };
    for placed in &view.placed {
        let slots = pass.slots(&placed.seen);
        let origin = (ox - placed.shift, oy + placed.y);
        if let Some(ops) = &placed.row.ops { emit(frame, ops, origin, slots, ids, &kept.cache.line(placed.seen.key), kept.resolver); }
        if placed.left_mark {
            crate::glyph(frame, left_glyph, ox + placed.x0, oy + placed.y, metrics[0], FaceId::new(face_base))?;
        }
    }
    let rows = &view.end.rows;
    fringe::paint(frame, snapshot, rows, &view.marks, fringe::Area { ox, oy, body_height, line: metrics[0].1, text_len: text.len(), reached_end: view.reached_end, face_base })?;
    // A row starting a numbered line shows its label; any other, blanks,
    // in the face of line numbers.
    if let Some(numbers) = &snapshot.line_numbers {
        for row in rows.iter().filter(|r| r.y < body_height && r.y + r.height > 0.0) {
            // A continuation row of point's line keeps its face, in blanks.
            let (x0, label, current) = gutter(snapshot, numbers_cell, row.start);
            let index = if current { numbers.current.0 } else { numbers.face.0 };
            let id = FaceId::new(face_base + index);
            let (ascent, height, cell, _) = metrics[index as usize];
            let blanks = " ".repeat(px::cells(x0 / cell));
            let mut at = 0.0;
            for c in label.unwrap_or(blanks.as_str()).chars() {
                let width = crate::advance(frame, id, c, cell);
                if at < px::of(snapshot.width) { crate::glyph_of(frame, c, ox + at, oy + row.y, width, (ascent, height), id)?; }
                at += width;
            }
        }
    }
    add_cursor(frame, snapshot, (ox, oy, body_height), cursor, view.cursor);
    // Each band's faces are the next table after the window's, so the
    // next window's faces start after them. The header and mode lines
    // span the whole window, margins and fringes included.
    let (left, right) = snapshot.chrome.insets();
    let (band_x, band_width) = (ox - px::of(left), px::of(left + snapshot.width + right));
    let mut copy = 0;
    if let Some(band) = &snapshot.header_line {
        copy += 1;
        let bounds = Rect::new(band_x, window_top, band_width, header_height);
        let stretches = (&snapshot.band_stretches.header_line[..], px::of(left));
        crate::paint_band(frame, snapshot, band, snapshot.header_line_face, bounds, stretches, GlyphRowRole::HeaderLine, (face_base, copy), (&metrics, &glyphless))?;
    }
    if let Some(band) = &snapshot.mode_line {
        copy += 1;
        let bounds = Rect::new(band_x, oy + body_height, band_width, px::of(snapshot.height) - header_height - body_height);
        let stretches = (&snapshot.band_stretches.mode_line[..], px::of(left));
        crate::paint_band(frame, snapshot, band, snapshot.mode_line_face, bounds, stretches, GlyphRowRole::ModeLine, (face_base, copy), (&metrics, &glyphless))?;
    }
    Ok(view.end)
}

/// The window's cursor at RECT, CURSOR saying whether the window is
/// selected and its colour, in a text area at OX, OY, BODY tall.
fn add_cursor(frame: &mut FrameGlyphBuffer, snapshot: &WindowSnapshot, (ox, oy, body_height): (f32, f32, f32), cursor: Option<(bool, Color)>, rect: Option<CursorRect>) {
    let window = DisplayWindowId::new(snapshot.id.cast_signed());
    if let (Some((active, color)), Some((x, y, width, height, ascent))) = (cursor, rect) {
        // No cursor may bleed into the mode line or another window.
        // With a right fringe, a cursor after a row that fills the text
        // area shows in the fringe (`overflow-newline-into-fringe`).
        let right = px::of(snapshot.width) + px::of(snapshot.chrome.right_fringe);
        if x >= 0.0 && x < right && y >= 0.0 && y + height <= body_height {
            use romacs_display_protocol::CursorStyle;
            use romacs_layout_input::CursorKind;
            let width = width.min(right - x);
            // A bar no wider than its glyph, an hbar no taller than its row
            // (GNU's draw_bar_cursor clamps both).
            let style = match if active { snapshot.cursor.selected } else { snapshot.cursor.unselected } {
                CursorKind::Box => Some(CursorStyle::FilledBox),
                CursorKind::Hollow => Some(CursorStyle::Hollow),
                CursorKind::Bar(thick) => Some(CursorStyle::Bar(px::of(thick).min(width))),
                CursorKind::Hbar(thick) => Some(CursorStyle::Hbar(px::of(thick).min(height))),
                CursorKind::None => None,
            };
            if let Some(style) = style {
                frame.add_cursor(window, ox + x, oy + y, width, height, style, color);
                if let Some(published) = frame.window_cursors.last_mut() {
                    published.active = active;
                    published.ascent = ascent;
                }
            }
        }
    }
}

/// The metrics the window gives the face at INDEX: PUBLISHED's when a
/// paint published its faces, else what a paint would publish.
fn window_metrics(pass: &mut Pass, index: usize, published: Option<&[Metrics]>, kept: &mut Kept) -> Result<Metrics, String> {
    if let Some(metrics) = published { return Ok(metrics[index]); }
    // A character of the bands or the text shows in nearly every face a
    // window names; only one that none does answers zeros.
    if index == 0 || pass.keeps(index) || pass.shown(kept)?[index].used { pass.metrics(index, kept) } else { Ok((0.0, 0.0, 0.0, 0.0)) }
}

/// The face ids a window's table takes in its frame: BASE its first, LEN
/// its length.
#[derive(Clone, Copy)]
struct Ids {
    base: u32,
    len: u32,
}

impl Ids {
    /// The id INK names: the face at its slot's table index (SLOTS), or
    /// that face's digit or box-line face, published above the table's
    /// band copies.
    fn of(self, ink: Ink, slots: &[u32]) -> FaceId {
        let derived = self.base + 3 * self.len;
        match ink {
            Ink::Face(slot) => FaceId::new(self.base + slots[slot as usize]),
            Ink::Digits(slot) => FaceId::new(derived + 2 * slots[slot as usize]),
            Ink::Line(slot) => FaceId::new(derived + 2 * slots[slot as usize] + 1),
        }
    }
}

/// Paint OPS, a row's glyphs, in FRAME with the row's origin at ORIGIN,
/// its line LINE's slots at the table indexes SLOTS.
fn emit(frame: &mut FrameGlyphBuffer, ops: &[Op], (ex, ey): (f32, f32), slots: &[u32], ids: Ids, line: &LineText, resolver: &mut FontResolver) {
    let fill = |fill: Fill| match fill {
        Fill::Background(slot) => line.faces[slot as usize].background,
        Fill::Foreground(slot) => line.faces[slot as usize].foreground,
        Fill::Pixel(pixel) => Color::from_pixel(pixel),
    };
    // The grid row of the row's top, where most of its glyphs are.
    let top = frame.slot_row(ey);
    for op in ops {
        match op {
            Op::Char { ch, x, y, dy, width, height, ascent, ink, edges } => {
                let row = if *y == 0.0 { top } else { frame.slot_row(ey + y) };
                frame.add_char_on_row(*ch, (ex + x, ey + y, row), (*width, *height, *ascent), ids.of(*ink, slots), *edges);
                if *dy != 0.0 && let Some(FrameGlyph::Char { y, baseline, .. }) = frame.glyphs.last_mut() { *y += dy; *baseline += dy; }
            }
            Op::Composed { text, ch, x, dy, width, height, ascent, slot, edges } => {
                let id = ids.of(Ink::Face(*slot), slots);
                crate::publish_selected_glyph(frame, resolver, id, text);
                frame.add_composed_char(text, *ch, ex + x, ey, *width, *height, *ascent, false);
                if let Some(FrameGlyph::Char { y, baseline, face_id, box_vertical_edges, .. }) = frame.glyphs.last_mut() {
                    (*face_id, *box_vertical_edges) = (id, *edges);
                    if *dy != 0.0 { *y += dy; *baseline += dy; }
                }
            }
            Op::Stretch { x, y, dy, width, height, fill: colour, ink, edges } => {
                frame.add_stretch(ex + x, ey + y, *width, *height, fill(*colour), ids.of(*ink, slots), false);
                if let Some(FrameGlyph::Stretch { y, box_vertical_edges, .. }) = frame.glyphs.last_mut() {
                    *box_vertical_edges = *edges;
                    if *dy != 0.0 { *y += dy; }
                }
            }
            Op::Image { image, x, y, dy, width, height, source, slot, face } => {
                frame.add_image(romacs_display_protocol::ImageId::new(*image), ex + x, ey + y, *width, *height);
                // The face's background under the image is its body's; the
                // slot's stretch paints the margins, where a relief goes.
                if let Some(FrameGlyph::Image { y, source_rect, slot_rect, box_rect, face_id, .. }) = frame.glyphs.last_mut() {
                    *source_rect = romacs_display_protocol::image::ImageSourceRect::new(source[0], source[1], source[2], source[3])
                        .unwrap_or(romacs_display_protocol::image::ImageSourceRect::FULL);
                    *slot_rect = Rect::new(ex + slot[0], ey + slot[1], slot[2], slot[3]);
                    *face_id = ids.of(Ink::Face(*face), slots);
                    if *dy != 0.0 { *y += dy; slot_rect.y += dy; box_rect.y += dy; }
                }
            }
            Op::Surface { surface, x, y, dy, width, height, face } => {
                frame.add_surface(romacs_display_protocol::SurfaceId::new(*surface), ex + x, ey + y, *width, *height);
                if let Some(FrameGlyph::Surface { y, face_id, .. }) = frame.glyphs.last_mut() {
                    *face_id = ids.of(Ink::Face(*face), slots);
                    if *dy != 0.0 { *y += dy; }
                }
            }
        }
    }
}

/// The context SEEN's line is laid out in for this window.
fn context_of(snapshot: &WindowSnapshot, frame: &Frame, pass: &mut Pass, seen: &Seen, numbers_cell: f32, kept: &mut Kept) -> Result<Context, String> {
    let hscroll = snapshot.hscroll;
    let shift = if seen.start == frame.point_line { hscroll.point_row } else { hscroll.rows };
    let shift = px::of(shift);
    // A slot whose face no character of the window shows in answers no
    // metrics; only a line with a slot that shows no character can have one.
    let mut ghosts = 0u64;
    if kept.cache.line(seen.key).ghostly {
        let slots = pass.slots(seen).to_vec();
        for (slot, &index) in slots.iter().enumerate().skip(1).take(63) {
            if !pass.keeps(index as usize) && !pass.shown(kept)?[index as usize].used { ghosts |= 1 << slot; }
        }
    }
    Ok(Context {
        width: px::of(snapshot.width),
        mode: if frame.truncate { Mode::Truncate } else if frame.word { Mode::Word } else { Mode::Window },
        last_x: frame.last_x, glyphs_mark: frame.glyphs_mark, special: frame.special, left_glyph: frame.left_glyph, dollar: frame.dollar,
        tab_width: snapshot.tab_width, x0: gutter(snapshot, numbers_cell, seen.start).0, number_cell: numbers_cell, numbered: snapshot.line_numbers.is_some(), shift, ghosts,
    })
}

/// Where a line's next row for a window comes from: the line's own row at
/// an index; a walk from a state, for the rows after a window start that
/// broke one; nowhere, after the line's last.
#[derive(Clone, Copy)]
enum Next { Kept(usize), Loose(Restart), Done }

/// A row got for a window, point on it when it is there, and what follows it.
struct Got {
    row: Wrapped,
    pointed: Option<Pointed>,
    next: Next,
}

/// One line being shown.
struct Shown {
    seen: Seen,
    layout: usize,
    context: Context,
    /// Point's byte in the line, when it is in it.
    point: Option<usize>,
    body: f32,
    paint: bool,
}

impl Shown {
    fn len(&self) -> usize { self.seen.end - self.seen.start }

    /// Point when it is on ROW: within it, or at the line's end on its last row.
    fn on(&self, row: &Wrapped) -> Option<usize> {
        self.point.filter(|&point| row.restart.at <= point && (point < row.end || (!row.continued && point == self.len())))
    }

    /// Walk the line: WITH is given the walk.
    fn walk<T>(&self, kept: &mut Kept, with: impl FnOnce(&mut Walk) -> T) -> T {
        let line = kept.cache.line(self.seen.key);
        let mut walk = Walk { line: &line, context: &self.context, faces: kept.faces, resolver: kept.resolver };
        with(&mut walk)
    }

    /// The line's own row INDEX at Y: as kept when all of it is in view,
    /// else walked for this place; none past the line's last row.
    fn kept(&self, kept: &mut Kept, index: usize, y: f32) -> Option<Got> {
        let row = kept.cache.row(self.seen.key, self.layout, index, false, kept.faces, kept.resolver)?;
        let next = if row.continued { Next::Kept(index + 1) } else { Next::Done };
        // A row clipped at the text area's top may lose glyphs and settle
        // otherwise: it is walked for its place.
        if self.body > 0.0 && y >= 0.0 {
            let row = if self.paint { kept.cache.row(self.seen.key, self.layout, index, true, kept.faces, kept.resolver)? } else { row };
            let pointed = self.on(&row).and_then(|point| self.probe(kept, &row.restart, point));
            return Some(Got { row, pointed, next });
        }
        let (row, _, pointed) = self.loose(kept, &row.restart, false, y);
        Some(Got { row, pointed, next })
    }

    /// Where POINT is on the row from RESTART, all of it in view.
    fn probe(&self, kept: &mut Kept, restart: &Restart, point: usize) -> Option<Pointed> {
        self.walk(kept, |walk| {
            let standing = walk.standing(restart, false);
            match walk.row(&standing, restart, Ask { point: Some(point), ..Ask::default() }) {
                Walked::Row { point, .. } => point,
                Walked::At(_) => None,
            }
        })
    }

    /// The row from RESTART at Y, walked for this place and kept nowhere:
    /// with what follows it and point when it is on it. MIXED: a glyph of
    /// another font's metrics was on the rows above the window's start.
    fn loose(&self, kept: &mut Kept, restart: &Restart, mixed: bool, y: f32) -> (Wrapped, Option<Restart>, Option<Pointed>) {
        let clip = Some(Clip { y, body: self.body });
        let point = self.point;
        self.walk(kept, |walk| {
            let standing = walk.standing(restart, mixed);
            let (row, next, pointed, _) = drawn(walk, &standing, restart, &Ask { clip, point, ..Ask::default() });
            (row, next, pointed)
        })
    }

    /// The row after a loose one that left NEXT: the line's own again when
    /// one of them starts there in that state.
    fn after(&self, kept: &mut Kept, next: Option<Restart>) -> Next {
        let Some(next) = next else { return Next::Done };
        match kept.cache.find(self.seen.key, self.layout, &next, kept.faces, kept.resolver) {
            Some(index) => Next::Kept(index),
            None => Next::Loose(next),
        }
    }

    /// The next row of the line at Y.
    fn next(&self, kept: &mut Kept, next: Next, y: f32) -> Option<Got> {
        match next {
            Next::Done => None,
            Next::Kept(index) => self.kept(kept, index, y),
            Next::Loose(restart) => {
                let (row, next, pointed) = self.loose(kept, &restart, false, y);
                Some(Got { row, pointed, next: self.after(kept, next) })
            }
        }
    }

    /// Point placed at the first byte of the row NEXT would give: a row
    /// below the text area, whose first character the frame-at-once loop
    /// still read.
    fn place_at(&self, kept: &mut Kept, next: Next) -> Option<PointPlace> {
        let restart = match next {
            Next::Kept(index) => kept.cache.row(self.seen.key, self.layout, index, false, kept.faces, kept.resolver)?.restart,
            Next::Loose(restart) => restart,
            Next::Done => return None,
        };
        self.point.filter(|&point| point == restart.at)?;
        self.probe(kept, &restart, restart.at).map(|pointed| pointed.place)
    }
}

/// A window's first row when its start is inside a line: how it began,
/// the row, and the y the rows above leave it at.
struct Start {
    started: Started,
    got: Got,
    y: f32,
}

/// The first row a window shows whose start is AT, inside SHOWN's line,
/// as the frame-at-once loop finds it: from the state the rows above the
/// start leave. TOP is the text area's top less the window's vscroll.
#[expect(clippy::float_cmp, reason = "layout coordinates are whole pixels, compared as the frame-at-once loop compared them")]
fn start_inside(shown: &Shown, kept: &mut Kept, at: usize, top: f32) -> Start {
    let (key, layout) = (shown.seen.key, shown.layout);
    let index = kept.cache.locate(key, layout, at, kept.faces, kept.resolver);
    let row = kept.cache.row(key, layout, index, false, kept.faces, kept.resolver).expect("a line has the row that holds its byte");
    let (mixed, raw_above) = kept.cache.above(key, layout, index);
    // The usual start: where a row of the line begins after a row with
    // some width, no glyph of another font's metrics above it (or a row
    // settled whatever is above).
    if row.restart.at == at && index > 0 {
        let held = kept.cache.row(key, layout, index - 1, false, kept.faces, kept.resolver).expect("the row above");
        if held.pen != held.x0 {
            if !mixed || row.settled.is_some() {
                let got = shown.kept(kept, index, top).expect("the row");
                return Start { started: Started::Broke, got, y: top };
            }
            let (row, next, pointed) = shown.loose(kept, &row.restart, true, top);
            let next = shown.after(kept, next);
            return Start { started: Started::Broke, got: Got { row, pointed, next }, y: top };
        }
    }
    // The state at the start, read off the row that holds it.
    let held = if row.restart.at == at && index > 0 { kept.cache.row(key, layout, index - 1, false, kept.faces, kept.resolver).expect("the row above") } else { row };
    let seeded = if row_is_above(&held, at) { kept.cache.above(key, layout, index - 1).0 } else { mixed };
    let standing = shown.walk(kept, |walk| {
        let from = walk.standing(&held.restart, seeded);
        match walk.row(&from, &held.restart, Ask { clip: Some(Clip { y: 0.0, body: 0.0 }), until: Some(at), ..Ask::default() }) {
            Walked::At(standing) => standing,
            Walked::Row { .. } => from,
        }
    });
    // Where no row is drawn before the text ends, the pen's y is where the
    // rows above left it.
    let raw_above = if row_is_above(&held, at) { raw_above - held.raw_height } else { raw_above };
    let point = shown.point;
    let restart = Restart { at, ..held.restart };
    let (row, next, pointed, started) = shown.walk(kept, |walk| {
        drawn(walk, &standing, &restart, &Ask { clip: Some(Clip { y: top, body: shown.body }), point, starting: true, ..Ask::default() })
    });
    let y = if started == Started::Never && shown.seen.newline.not() { top + raw_above } else { top };
    let (row, pointed) = if y == top { (row, pointed) } else {
        // The pen's y differs from where it was walked: walk it there.
        let (row, _, pointed, _) = shown.walk(kept, |walk| drawn(walk, &standing, &restart, &Ask { clip: Some(Clip { y, body: shown.body }), point, starting: true, ..Ask::default() }));
        (row, pointed)
    };
    let next = shown.after(kept, next);
    Start { started, got: Got { row, pointed, next }, y }
}

/// Whether HELD, the row a start state is read off, ends at AT: the row
/// above the one AT begins.
fn row_is_above(held: &Wrapped, at: usize) -> bool { held.end == at && held.continued }

/// The rows SNAPSHOT's window shows from its start, in the window's
/// inputs FRAME; with their glyphs when PAINT.
#[allow(clippy::too_many_lines, reason = "the frame-at-once loop's view of the rows, kept in its order")]
fn rows(snapshot: &WindowSnapshot, frame: &Frame, pass: &mut Pass, numbers_cell: f32, default: Metrics, paint: bool, kept: &mut Kept) -> Result<View, String> {
    let text_len = snapshot.text.utf8.len();
    let body = frame.body_height;
    let top = -px::of(snapshot.vscroll_px);
    let mut y = top;
    let mut end_byte = snapshot.start_byte;
    let mut visible_end = snapshot.start_byte;
    let fully_visible = |y: f32, height: f32| y >= 0.0 && y + height <= body;
    let (mut rows, mut marks, mut placed) = (vec![], vec![], vec![]);
    let (mut place, mut cursor): (Option<PointPlace>, Option<CursorRect>) = (None, None);
    let mut n = 0;
    let reached_end = 'lines: loop {
        let Some(seen) = pass.line(n, kept)? else { break 'lines end_byte == text_len };
        n += 1;
        let context = context_of(snapshot, frame, pass, &seen, numbers_cell, kept)?;
        let layout = kept.cache.layout(seen.key, &context);
        let in_line = (seen.start..=seen.end).contains(&snapshot.point_byte).then(|| snapshot.point_byte - seen.start);
        let shown = Shown { seen, layout, context, point: in_line, body, paint };
        let wrapped = kept.cache.counts.rows_wrapped;
        let at = (snapshot.start_byte > seen.start).then(|| snapshot.start_byte - seen.start);
        let (mut first, mut next) = (None, Next::Kept(0));
        let mut row_start = seen.start;
        if let Some(at) = at {
            let start = start_inside(&shown, kept, at, top);
            y = start.y;
            row_start = snapshot.start_byte;
            if start.y >= body {
                // Nothing shows: what the loop read before it saw that.
                match start.started {
                    Started::Row | Started::InPlace => {}
                    Started::Broke => place = start.got.pointed.filter(|_| in_line == Some(at)).map(|p| p.place),
                    Started::Forced => {
                        end_byte = snapshot.start_byte + snapshot.text.utf8[snapshot.start_byte..].chars().next().map_or(0, char::len_utf8);
                        place = start.got.pointed.filter(|_| in_line == Some(at)).map(|p| p.place);
                    }
                    Started::Never => {
                        place = start.got.pointed.filter(|_| !seen.newline || in_line != Some(shown.len())).map(|p| p.place);
                        if !seen.newline { end_byte = text_len; }
                    }
                }
                count(kept, wrapped);
                break 'lines end_byte == text_len;
            }
            first = Some((start.got, true));
        } else if y >= body {
            // The line's first character is read and nothing more.
            if seen.end == seen.start && !seen.newline && snapshot.point_byte == text_len {
                place = shown.probe(kept, &Restart::start((default.0, default.1)), 0).map(|p| p.place);
            }
            count(kept, wrapped);
            break 'lines end_byte == text_len;
        }
        loop {
            let (got, starts) = match first.take() {
                Some(first) => first,
                None => match shown.next(kept, next, y) { Some(got) => (got, false), None => break },
            };
            let Got { row, pointed, next: after } = got;
            next = after;
            let last = !row.continued;
            let ends_text = last && !seen.newline;
            let row_end = if !last { seen.start + row.end } else if seen.newline { seen.end + 1 } else { text_len };
            if visible_end == row_start && fully_visible(y, row.raw_height) {
                visible_end = if ends_text { text_len + 1 } else { row_end };
            }
            end_byte = row_end;
            if pointed.is_some() { place = pointed.map(|p| p.place); }
            if !ends_text || y < body {
                let shift = context.shift;
                let scrolled = shift > 0.0 && (seen.newline || row_start < text_len);
                rows.push(Row { start: row_start, y, height: row.height });
                marks.push(RowMarks { continued: !last, continuation: starts || row.restart.continuation, truncated: last && row.truncated,
                    truncated_left: last && scrolled });
                let marked = last && scrolled && left_mark(frame, shift) > 0.0 && y + default.1 > 0.0;
                if let Some(pointed) = pointed {
                    let (dy, height) = row.settled.map_or((0.0, pointed.rect.2), |ascent| settled_cursor(pointed.rect, ascent, row.height));
                    cursor = Some((pointed.rect.0, y + dy, pointed.rect.1, height, pointed.rect.3));
                }
                if paint { placed.push(Placed { seen, y, shift, x0: row.x0, left_mark: marked, row: row.clone() }); }
            } else if let Some(pointed) = pointed {
                cursor = Some((pointed.rect.0, y, pointed.rect.1, pointed.rect.2, pointed.rect.3));
            }
            y += row.height;
            if ends_text { count(kept, wrapped); break 'lines true; }
            if last { break; }
            row_start = row_end;
            if y >= body {
                // The character the next row starts with is read: point
                // there is placed.
                if let Some(at) = shown.place_at(kept, next) { place = Some(at); }
                count(kept, wrapped);
                break 'lines false;
            }
        }
        count(kept, wrapped);
    };
    Ok(View { end: WindowEnd { id: snapshot.id, end: end_byte, visible_end, body_height: body, rows, point: place }, marks, placed, cursor, reached_end })
}

/// The advance of a scrolled row's left truncation glyph.
fn left_mark(frame: &Frame, shift: f32) -> f32 { if shift > 0.0 { frame.dollar } else { 0.0 } }

/// Count the line a window just showed: laid out when a row of it was
/// wrapped since BEFORE, else served.
fn count(kept: &mut Kept, before: u64) {
    let counts = &mut kept.cache.counts;
    if counts.rows_wrapped > before { counts.lines_laid += 1; } else { counts.lines_served += 1; }
}
