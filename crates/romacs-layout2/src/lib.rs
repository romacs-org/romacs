//! G1a logical-line layout over owned Seam L data. No evaluator can enter this pass.
#[cfg(test)]
#[path = "selection_order_tests.rs"]
mod selection_order_tests;
#[cfg(test)]
mod alternative_state_tests;
use romacs_display_protocol::frame_glyphs::GlyphRowRole;
use romacs_display_protocol::{
    Color, DisplayFrameId, DisplayWindowId, FaceId, FrameGlyphBuffer, ParentFrameRect,
    PresentationId, PresentedFramePlacement, Rect,
};
use romacs_font::{FontRequest, FontResolver, FontsetPolicy, publish_face_text};
use romacs_layout_input::{TextSlice, WindowSnapshot, FrameSnapshot, Face, MeasureInput, Measured, LineWrap, tab_advance};

/// Resolve the actual default monospace geometry before window sizing.
pub fn frame_metrics(face: &Face) -> Result<(u32, u32), String> {
    let snapshot = WindowSnapshot {
        id: 1, width: 1, height: 1, start_byte: 0, point_byte: 0, buffer_version: 0,
        text: TextSlice { utf8: String::new(), face_runs: vec![] }, mode_line: None,
        mode_line_face: romacs_layout_input::FaceHandle(0), vscroll_px: 0, tab_width: 8, wrap: Default::default(), faces: vec![face.clone()],
    };
    let frame = layout(&snapshot, 1)?;
    Ok((frame.char_height.ceil() as u32, frame.char_width.ceil() as u32))
}

/// Stateful font discovery cache. Contains only owned font data, never VM values.
pub struct LayoutEngine { resolver: FontResolver }
impl LayoutEngine {
    pub fn new() -> Result<Self, String> {
        Ok(Self { resolver: FontResolver::new().map_err(|e| e.to_string())? })
    }
    pub fn layout_frame(&mut self, snapshot: &FrameSnapshot, presentation: u64, cursor: Option<(u64, Color)>) -> Result<(FrameGlyphBuffer, Vec<(u64, usize)>), String> {
        layout_frame_inner(snapshot, presentation, cursor, &mut self.resolver)
    }
    /// The host layout hook: INPUT's geometry as painting would lay it out.
    pub fn measure(&mut self, input: &MeasureInput) -> Result<Measured, String> {
        measure_inner(input, &mut self.resolver)
    }
    pub fn layout(&mut self, snapshot: &WindowSnapshot, presentation: u64) -> Result<FrameGlyphBuffer, String> {
        let mut frame = new_frame(snapshot.width, snapshot.height, presentation)?;
        frame.background = color(snapshot.faces.first().ok_or("missing face")?.background);
        append_window(snapshot, &mut frame, &mut self.resolver, 0.0, 0.0, None)?;
        Ok(frame)
    }
}

/// Stateless convenience for fixtures; interactive hosts retain a LayoutEngine.
/// One publication for every VM-produced leaf. Commit ends only after paint.
pub fn layout_frame(snapshot: &FrameSnapshot, presentation: u64) -> Result<(FrameGlyphBuffer, Vec<(u64, usize)>), String> {
    LayoutEngine::new()?.layout_frame(snapshot, presentation, None)
}

/// Publish point using the same shaped advances and row metrics as text.
/// Selection and color are evaluator-owned policy; this pass owns geometry.
pub fn layout_frame_with_cursors(snapshot: &FrameSnapshot, presentation: u64, selected: u64, cursor_color: Color) -> Result<(FrameGlyphBuffer, Vec<(u64, usize)>), String> {
    LayoutEngine::new()?.layout_frame(snapshot, presentation, Some((selected, cursor_color)))
}

fn layout_frame_inner(snapshot: &FrameSnapshot, presentation: u64, cursor: Option<(u64, Color)>, resolver: &mut FontResolver) -> Result<(FrameGlyphBuffer, Vec<(u64, usize)>), String> {
    resolver.set_selection_state(snapshot.font_selection);
    resolver.set_alternative_state(snapshot.font_alternatives.clone());
    let first = snapshot.windows.first().ok_or("empty frame")?;
    let mut frame = new_frame(snapshot.width, snapshot.height, presentation)?;
    frame.background = color(first.snapshot.faces.first().ok_or("missing face")?.background);
    let mut ends = Vec::new();
    let mut ids = std::collections::HashSet::new();
    // An EMPTY extent draws nothing, so it cannot overflow the frame and its
    // origin is not meaningful: GNU clamps a body width to
    // `max(width / denom, 0)` (src/window.c:1136-1137) and keeps
    // redisplaying, and a window whose margins exceed its own width legally
    // has its body origin outside the frame. Requiring such a body to fit
    // rejected the whole frame and the error propagated out of the GUI loop,
    // killing redisplay. A NON-empty extent must still fit, and arithmetic
    // overflow is still refused in both cases.
    let fits = |origin: u32, extent: u32, limit: u32| match origin.checked_add(extent) {
        None => false,
        Some(end) => extent == 0 || end <= limit,
    };
    for placed in &snapshot.windows {
        if !ids.insert(placed.snapshot.id)
            || !fits(placed.x, placed.snapshot.width, snapshot.width)
            || !fits(placed.y, placed.snapshot.height, snapshot.height) {
            return Err("invalid or duplicate placed window".into());
        }
        // Windows with a mode line always own a cursor; the minibuffer (no mode
        // line) only while it is the selected window, i.e. a prompt is active.
        let end = append_window(&placed.snapshot, &mut frame, resolver, placed.x as f32, placed.y as f32, cursor.filter(|&(id, _)| placed.snapshot.mode_line.is_some() || id == placed.snapshot.id).map(|(id, color)| (id == placed.snapshot.id, color)))?;
        ends.push((placed.snapshot.id, end));
    }
    Ok((frame, ends))
}

fn new_frame(width: u32, height: u32, presentation: u64) -> Result<FrameGlyphBuffer, String> {
    if width == 0 || height == 0 || presentation == 0 { return Err("invalid frame extent or presentation".into()); }
    let mut frame = FrameGlyphBuffer::with_size(width as f32, height as f32);
    frame.presentation_id = PresentationId::new(presentation);
    frame.frame_placement = PresentedFramePlacement::new(
        DisplayFrameId::new(1), frame.presentation_id, None,
        ParentFrameRect::new(0.0, 0.0, frame.width, frame.height).map_err(|e| e.to_string())?, 0,
    );
    Ok(frame)
}

fn validate_runs(slice: &TextSlice, face_count: usize) -> Result<(), String> {
    let mut end = 0;
    for run in &slice.face_runs {
        if run.bytes.start < end
            || run.bytes.start > run.bytes.end
            || !slice.utf8.is_char_boundary(run.bytes.start)
            || !slice.utf8.is_char_boundary(run.bytes.end)
            || run.face.0 as usize >= face_count
        {
            return Err("invalid, overlapping or non-UTF8 face run".into());
        }
        end = run.bytes.end;
    }
    Ok(())
}

fn color(rgb: [u8; 3]) -> Color {
    Color::from_pixel((u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]))
}

/// LTR rows, continued or truncated as GNU does without fringes; no cursor or policy evaluation.
/// Byte positions are relative to the owned UTF-8 text, not VM heap references.
pub fn layout(snapshot: &WindowSnapshot, presentation: u64) -> Result<FrameGlyphBuffer, String> {
    let mut frame = new_frame(snapshot.width, snapshot.height, presentation)?;
    frame.background = color(snapshot.faces.first().ok_or("missing face")?.background);
    let mut resolver = FontResolver::new().map_err(|e| e.to_string())?;
    append_window(snapshot, &mut frame, &mut resolver, 0.0, 0.0, None)?;
    Ok(frame)
}

fn append_window(snapshot: &WindowSnapshot, frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver, ox: f32, oy: f32, cursor: Option<(bool, Color)>) -> Result<usize, String> {
    let text = &snapshot.text.utf8;
    if snapshot.height == 0
        || snapshot.faces.is_empty()
        || !text.is_char_boundary(snapshot.start_byte)
        || !text.is_char_boundary(snapshot.point_byte)
    {
        return Err("invalid snapshot dimensions, positions, face table or presentation".into());
    }
    // A body width of zero is legal, and refusing it would kill redisplay.
    //
    // `width` here is already the *body* width: the publisher subtracts the
    // margins (romacs-vm/src/window.rs). GNU validates margins once against
    // the window they are assigned to (src/window.c:8477), copies them to the
    // new window on split without re-validating (:5610-5612), and clamps the
    // resulting body width at zero under an explicit "Don't return a negative
    // value" (:1136-1137) -- redisplay continues. Measured on a 640px root at
    // an 8px cell: margins (30 . 30) leave a 160px body, and splitting into two
    // 320px windows leaves both bodies at 0 with redisplay still returning.
    //
    // So an empty body paints nothing and displays nothing, which makes
    // window-end equal window-start. Returning Err here instead propagated out
    // of the GUI loop and terminated redisplay -- the failure mode s121 was
    // repaired to prevent. A window whose *total* geometry does not fit its
    // parent is still rejected, by layout_frame's placement check.
    if snapshot.width == 0 {
        return Ok(snapshot.start_byte);
    }
    validate_runs(&snapshot.text, snapshot.faces.len())?;
    if let Some(band) = &snapshot.mode_line {
        validate_runs(band, snapshot.faces.len())?;
        if snapshot.mode_line_face.0 as usize >= snapshot.faces.len()
            || band.utf8.chars().any(char::is_control)
        {
            return Err("invalid mode-line face or control character".into());
        }
    }
    let face_base = u32::try_from(frame.faces.len()).map_err(|e| e.to_string())?;
    let mut slices = vec![(&snapshot.text, 0)];
    if let Some(band) = &snapshot.mode_line { slices.push((band, snapshot.mode_line_face.0 as usize)); }
    let face_text = face_text(snapshot.faces.len(), &slices);
    let metrics = realize_faces(&snapshot.faces, &face_text, frame, resolver, face_base)?;
    if face_base == 0 { frame.char_height = metrics[0].1; frame.char_width = metrics[0].2; }
    let band_height = snapshot.mode_line.as_ref().map_or(0.0, |band| {
        std::iter::once(snapshot.mode_line_face.0 as usize)
            .chain(band.face_runs.iter().map(|run| run.face.0 as usize))
            .map(|index| metrics[index].1)
            .fold(0.0_f32, f32::max)
    });
    let body_height = (snapshot.height as f32 - band_height).max(0.0);
    frame.set_draw_context(
        DisplayWindowId::new(i64::try_from(snapshot.id).map_err(|e| e.to_string())?),
        GlyphRowRole::Text,
        Some(Rect::new(ox, oy, snapshot.width as f32, body_height)),
    );
    // GNU on a frame without fringes: a row ends where its next glyph
    // would pass the text area less the continuation (`\\') or
    // truncation (`$') glyph, which it shows there (display_line,
    // move_it_in_display_line_to). Rows are laid out from the start of
    // the window start's line, so a window starting on a continuation
    // line breaks its rows, and stops its TABs, as from the line start.
    let truncate = snapshot.wrap == LineWrap::Truncate;
    let special = if truncate { '$' } else { '\\' };
    let default_id = FaceId::new(face_base);
    let last_x = snapshot.width as f32 - advance(frame, default_id, special, metrics[0].2);
    let line_start = text[..snapshot.start_byte].rfind('\n').map_or(0, |at| at + 1);
    let mut x = 0.0;
    let mut y = -(snapshot.vscroll_px as f32);
    let mut row_height = metrics[0].1;
    let mut end_byte = snapshot.start_byte;
    let mut point_rect = None;
    // The width of the rows of this line above this one, and the glyphs
    // on this row; past the truncation glyph, the rest of the line is hidden.
    let (mut continued, mut hpos, mut hidden) = (0.0f32, 0usize, false);
    let mut drawing = line_start == snapshot.start_byte;
    for (byte, ch) in text[line_start..].char_indices().map(|(at, ch)| (line_start + at, ch)) {
        if !drawing && byte >= snapshot.start_byte {
            drawing = true;
            (x, y, row_height, hpos) = (0.0, -(snapshot.vscroll_px as f32), metrics[0].1, 0);
        }
        if drawing && y >= body_height { break; }
        if drawing { end_byte = byte + ch.len_utf8(); }
        let at_point = drawing && byte == snapshot.point_byte;
        if at_point { point_rect = (!hidden).then_some((x, y, metrics[0].2, row_height, metrics[0].0)); }
        if ch == '\n' {
            x = 0.0;
            y += row_height;
            row_height = metrics[0].1;
            (continued, hpos, hidden) = (0.0, 0, false);
            continue;
        }
        if hidden { continue; }
        let index = face_at(&snapshot.text, byte, 0);
        let id = FaceId::new(face_base + index as u32);
        let (ascent, height, cell, space) = metrics[index];
        let shown = shown(ch);
        let width = |frame: &FrameGlyphBuffer, x: f32| match &shown {
            Shown::Tab => tab_advance(f64::from(x), f64::from(space), snapshot.tab_width) as f32,
            Shown::Glyphs(glyphs) => glyphs.chars().map(|glyph| advance(frame, id, glyph, cell)).sum(),
        };
        let mut advance_px = width(frame, x + continued);
        if !truncate && hpos > 0 && x + advance_px > last_x {
            if drawing && y < body_height && y + metrics[0].1 > 0.0 { glyph(frame, special, ox + x, oy + y, metrics[0], default_id)?; }
            // A TAB that does not fit continues from the text area's edge.
            continued += if matches!(shown, Shown::Tab) { last_x } else { x };
            (x, y, row_height, hpos) = (0.0, y + row_height, metrics[0].1, 0);
            advance_px = width(frame, continued);
            if at_point { point_rect = Some((x, y, metrics[0].2, row_height, metrics[0].0)); }
        }
        row_height = row_height.max(height);
        let visible = drawing && body_height > 0.0 && y < body_height && y + height > 0.0 && x < snapshot.width as f32;
        if truncate && x + advance_px > last_x {
            if visible { glyph(frame, special, ox + x, oy + y, metrics[0], default_id)?; }
            hidden = true;
            if at_point { point_rect = Some((x, y, metrics[0].2, row_height, ascent)); }
            continue;
        }
        let cursor = match &shown {
            Shown::Tab => {
                if visible { frame.add_stretch(ox + x, oy + y, advance_px, height, color(snapshot.faces[index].background), id, false); }
                // GNU's cursor on a stretch is no wider than a column
                // (`x-stretch-cursor' nil).
                advance_px.min(metrics[0].2)
            }
            Shown::Glyphs(glyphs) => {
                let mut at = x;
                for c in glyphs.chars() {
                    if visible && at < snapshot.width as f32 { glyph(frame, c, ox + at, oy + y, (ascent, height, cell, space), id)?; }
                    at += advance(frame, id, c, cell);
                }
                advance(frame, id, glyphs.chars().next().unwrap_or(ch), cell)
            }
        };
        if at_point { point_rect = Some((x, y, cursor, row_height, ascent)); }
        x += advance_px;
        hpos += 1;
    }
    if snapshot.point_byte == text.len() && end_byte == text.len() {
        point_rect = Some((x, y, metrics[0].2, row_height, metrics[0].0));
    }
    if let (Some((active, color)), Some((x, y, width, height, ascent))) = (cursor, point_rect) {
        // No cursor may bleed into the mode line or another window.
        if x >= 0.0 && x < snapshot.width as f32 && y >= 0.0 && y + height <= body_height {
            frame.add_cursor(DisplayWindowId::new(snapshot.id as i64), ox + x, oy + y,
                width.min(snapshot.width as f32 - x), height,
                if active { romacs_display_protocol::CursorStyle::FilledBox } else { romacs_display_protocol::CursorStyle::Hollow }, color);
            let published = frame.window_cursors.last_mut().unwrap();
            published.active = active;
            published.ascent = ascent;
        }
    }
    if let Some(band) = &snapshot.mode_line {
        let default = snapshot.mode_line_face.0 as usize;
        let bounds = Rect::new(ox, oy + body_height, snapshot.width as f32, snapshot.height as f32 - body_height);
        let background = color(snapshot.faces[default].background);
        frame.add_background(bounds.x, bounds.y, bounds.width, bounds.height, background);
        frame.set_draw_context(
            DisplayWindowId::new(i64::try_from(snapshot.id).map_err(|e| e.to_string())?),
            GlyphRowRole::ModeLine,
            Some(bounds),
        );
        let offset = u32::try_from(snapshot.faces.len()).map_err(|e| e.to_string())?;
        for index in 0..snapshot.faces.len() {
            let source = FaceId::new(face_base + index as u32);
            let target = FaceId::new(face_base.checked_add(offset).and_then(|v| v.checked_add(index as u32)).ok_or("too many faces")?);
            let mut face = frame.faces[&source].clone();
            face.id = target;
            face.background = background;
            frame.faces.insert(target, face);
            if let Some(chars) = frame.char_fonts.get(&source).cloned() {
                frame.char_fonts.insert(target, chars);
            }
        }
        let mut x = 0.0;
        for (byte, ch) in band.utf8.char_indices() {
            let index = face_at(band, byte, default);
            let id = FaceId::new(face_base + offset + index as u32);
            let (ascent, height, cell, _) = metrics[index];
            let advance = advance(frame, id, ch, cell);
            if x < snapshot.width as f32 {
                frame.add_char(ch, ox + x, bounds.y, advance, height, ascent, false);
                *frame
                    .glyphs
                    .last_mut()
                    .and_then(|g| g.face_id_mut())
                    .ok_or("missing mode-line face")? = id;
            }
            x += advance;
        }
    }
    Ok(end_byte)
}

/// Draw CH at X, Y in face ID, whose ascent, height and nominal advance
/// METRICS gives.
fn glyph(frame: &mut FrameGlyphBuffer, ch: char, x: f32, y: f32, metrics: (f32, f32, f32, f32), id: FaceId) -> Result<(), String> {
    let (ascent, height, cell, _) = metrics;
    let width = advance(frame, id, ch, cell);
    frame.add_char(ch, x, y, width, height, ascent, false);
    *frame.glyphs.last_mut().and_then(|g| g.face_id_mut()).ok_or("missing character face")? = id;
    Ok(())
}

/// How a buffer character other than newline is drawn: a TAB as a
/// stretch, another control character as GNU's escape (`^X`, or `\NNN`
/// for one beyond ASCII), anything else as itself.
enum Shown { Tab, Glyphs(String) }

fn shown(ch: char) -> Shown {
    match ch {
        '\t' => Shown::Tab,
        '\u{0}'..='\u{1f}' | '\u{7f}' => Shown::Glyphs(format!("^{}", char::from((ch as u8) ^ 0x40))),
        _ if ch.is_control() => Shown::Glyphs(format!("\\{:03o}", ch as u32)),
        _ => Shown::Glyphs(ch.to_string()),
    }
}

/// The face table index painting SLICE's character at BYTE.
fn face_at(slice: &TextSlice, byte: usize, default: usize) -> usize {
    slice.face_runs.iter().find(|run| run.bytes.contains(&byte)).map_or(default, |run| run.face.0 as usize)
}

/// Each face's painted characters. Publishing the whole window for every
/// color-only face multiplies Unicode bindings by the face-table size,
/// even when those faces are unused.
fn face_text(faces: usize, slices: &[(&TextSlice, usize)]) -> Vec<String> {
    let mut text = vec![String::new(); faces];
    // The continuation and truncation glyphs, in the default face.
    text[0].push_str("\\$");
    for (slice, default) in slices {
        for (byte, ch) in slice.utf8.char_indices() {
            let face = &mut text[face_at(slice, byte, *default)];
            match shown(ch) {
                Shown::Tab => face.push(' '),
                Shown::Glyphs(glyphs) => face.push_str(&glyphs),
            }
        }
    }
    text
}

/// Publish FACES into FRAME from FACE_BASE on and resolve the fonts their
/// characters need: each face's ascent, row height, nominal advance and
/// space width.
fn realize_faces(faces: &[Face], face_text: &[String], frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver, face_base: u32) -> Result<Vec<(f32, f32, f32, f32)>, String> {
    let policy = FontsetPolicy::new();
    let mut metrics = Vec::new();
    for (index, face) in faces.iter().enumerate() {
        if face.family.is_empty() || !face.pixel_size.is_finite() || face.pixel_size <= 0.0 {
            return Err("invalid font request".into());
        }
        let id = FaceId::new(face_base.checked_add(u32::try_from(index).map_err(|e| e.to_string())?).ok_or("too many faces")?);
        frame.set_face_with_font(
            id,
            color(face.foreground),
            Some(color(face.background)),
            &face.family,
            400,
            false,
            face.pixel_size,
            0,
            None,
            0,
            None,
            0,
            None,
            false,
        );
        let published = publish_face_text(
            resolver,
            frame,
            id,
            &FontRequest::new(&face.family, face.pixel_size),
            &policy,
            &face_text[index],
        )
        .map_err(|e| e.to_string())?;
        if !published.unresolved.is_empty() {
            return Err(format!("unresolved face text: {:?}", published.unresolved));
        }
        let font = &frame.fonts[&published.primary];
        let ascent = font.ascent_px;
        let height = (font.ascent_px + font.descent_px).ceil();
        let cell = font.glyph_advance.cell_advance_px().unwrap_or(font.space_advance_px);
        let space = font.space_advance_px;
        let per_glyph = font.glyph_advance.cell_advance_px().is_none();
        if !cell.is_finite() || cell <= 0.0 { return Err("font has no positive nominal advance".into()); }
        if per_glyph {
            // Dual-width fonts such as Iosevka report per-glyph advances. The
            // nominal space is only the frame column metric; text uses the
            // resolver's exact cached advance, including ASCII.
            let request = FontRequest::new(&face.family, face.pixel_size);
            let mut ascii: Vec<_> = face_text[index].chars().filter(|c| c.is_ascii() && !c.is_control()).collect();
            ascii.sort_unstable(); ascii.dedup();
            for ch in ascii {
                let resolved = resolver.resolve_char(&request, ch, &policy).map_err(|e| e.to_string())?;
                frame.fonts.insert(resolved.font.id, resolved.font);
                frame.char_fonts.entry(id).or_default().insert(ch, resolved.glyph);
            }
        }
        metrics.push((ascent, height, cell, space));
    }
    Ok(metrics)
}

/// The advance painting CH in face ID takes: its resolved glyph's, else
/// the face's nominal CELL.
fn advance(frame: &FrameGlyphBuffer, id: FaceId, ch: char, cell: f32) -> f32 {
    frame.char_fonts.get(&id).and_then(|chars| chars.get(&ch)).map_or(cell, |glyph| glyph.advance_px)
}

/// Lay INPUT out as `append_window` paints its text, without presenting
/// it: the advance and face row height of each character, from the same
/// face realization and advances. A TAB answers its face's space, from
/// which the caller makes its stretch (`tab_advance`); another control
/// character, its escape's glyphs.
fn measure_inner(input: &MeasureInput, resolver: &mut FontResolver) -> Result<Measured, String> {
    resolver.set_selection_state(input.font_selection);
    resolver.set_alternative_state(input.font_alternatives.clone());
    if input.faces.is_empty() { return Err("missing face".into()); }
    validate_runs(&input.text, input.faces.len())?;
    let mut frame = new_frame(1, 1, 1)?;
    let face_text = face_text(input.faces.len(), &[(&input.text, 0)]);
    let metrics = realize_faces(&input.faces, &face_text, &mut frame, resolver, 0)?;
    let pixels = |value: f32| if value.fract() == 0.0 && value >= 0.0 && value <= u32::MAX as f32 { Ok(value as u32) }
        else { Err(format!("fractional or negative advance {value}")) };
    let face_heights = metrics.iter().map(|m| pixels(m.1)).collect::<Result<Vec<_>, _>>()?;
    let mut glyphs = Vec::with_capacity(input.text.utf8.len());
    for (byte, ch) in input.text.utf8.char_indices() {
        if ch == '\n' { glyphs.push((0, face_heights[0])); continue; }
        let index = face_at(&input.text, byte, 0);
        let (_, _, cell, space) = metrics[index];
        let width = match shown(ch) {
            Shown::Tab => space,
            Shown::Glyphs(shown) => shown.chars().map(|glyph| advance(&frame, FaceId::new(index as u32), glyph, cell)).sum(),
        };
        glyphs.push((pixels(width)?, face_heights[index]));
    }
    Ok(Measured { glyphs, face_heights })
}
