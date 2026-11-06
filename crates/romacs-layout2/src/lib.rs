//! G1a logical-line layout over owned Seam L data. No evaluator can enter this pass.
#[cfg(test)]
#[path = "selection_order_tests.rs"]
mod selection_order_tests;
#[cfg(test)]
mod alternative_state_tests;
mod fringe;
mod glyphless;
use glyphless::Glyphless;
use fringe::RowMarks;
use romacs_display_protocol::frame_glyphs::GlyphRowRole;
use romacs_display_protocol::{
    Color, DisplayFrameId, DisplayWindowId, FaceId, FrameGlyphBuffer, ParentFrameRect,
    PresentationId, PresentedFramePlacement, Rect,
};
use romacs_font::{FontRequest, FontResolver, publish_face_text};
use romacs_layout_input::{NoFont, ChildFrameSnapshot, STATUS_BAR_ACCENT, STATUS_BAR_PAD, StatusBarSnapshot, TabBarSnapshot, TextSlice, WindowSnapshot, FrameSnapshot, Face, FaceHandle, MeasureInput, Measured, LineWrap, PointPlace, Row, WindowEnd, StretchWidth, stretch_advance, tab_advance};

/// Resolve the actual default monospace geometry before window sizing.
pub fn frame_metrics(face: &Face) -> Result<(u32, u32), String> {
    let frame = layout(&metrics_snapshot(face), 1)?;
    Ok((frame.char_height.ceil() as u32, frame.char_width.ceil() as u32))
}

/// An empty window in FACE alone: what the frame's character cell is.
fn metrics_snapshot(face: &Face) -> WindowSnapshot {
    WindowSnapshot { underline_rules: Default::default(), band_stretches: Default::default(),
        id: 1, width: 1, height: 1, start_byte: 0, point_byte: 0, buffer_version: 0,
        text: TextSlice { extends: vec![], utf8: String::new(), face_runs: vec![], raw: vec![] }, mode_line: None,
        mode_line_face: romacs_layout_input::FaceHandle(0), header_line: None, header_line_face: romacs_layout_input::FaceHandle(0),
        vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap: Default::default(), cursor_off: false, stretches: vec![], images: vec![], wrap_flips: vec![], line_numbers: None, chrome: Default::default(), faces: vec![face.clone()],
    }
}

/// Stateful font discovery cache and the last frame laid out. Contains only
/// owned font and glyph data, never VM values.
pub struct LayoutEngine { resolver: FontResolver, last: Option<Laid> }

/// A frame laid out with every cursor, as if none were turned off.
struct Laid {
    snapshot: FrameSnapshot,
    cursor: Option<(u64, Color)>,
    frame: FrameGlyphBuffer,
    ends: Vec<WindowEnd>,
}
impl LayoutEngine {
    pub fn new() -> Result<Self, String> {
        Ok(Self { resolver: FontResolver::new().map_err(|e| e.to_string())?, last: None })
    }
    /// SNAPSHOT laid out at PRESENTATION. A frame that differs from the last
    /// one only in which cursors are off (GNU's blink, `internal-show-cursor')
    /// is not laid out again: the last layout shows it, less those cursors.
    pub fn layout_frame(&mut self, snapshot: &FrameSnapshot, presentation: u64, cursor: Option<(u64, Color)>) -> Result<(FrameGlyphBuffer, Vec<WindowEnd>), String> {
        let reusable = self.last.as_ref().is_some_and(|last| last.cursor == cursor && last.snapshot.lays_out_like(snapshot));
        if !reusable {
            let (mut frame, ends) = layout_frame_inner(snapshot, presentation, cursor, &mut self.resolver)?;
            frame.layout_identity = next_layout_identity();
            self.last = Some(Laid { snapshot: snapshot.clone(), cursor, frame, ends });
        }
        let last = self.last.as_ref().expect("laid out above");
        let mut frame = last.frame.clone();
        stamp(&mut frame, presentation)?;
        frame.background_alpha = snapshot.background_alpha;
        hide_cursors_off(&mut frame, snapshot);
        Ok((frame, last.ends.clone()))
    }
    /// SNAPSHOT's child frames laid out at PRESENTATION, each its own frame
    /// placed over the root frame, lowest first, and their windows' ends.
    /// The root frame's layout never depends on them.
    pub fn layout_children(&mut self, snapshot: &FrameSnapshot, presentation: u64, cursor: Option<(u64, Color)>) -> Result<(Vec<FrameGlyphBuffer>, Vec<WindowEnd>), String> {
        let mut frames = Vec::with_capacity(snapshot.children.len());
        let mut ends = Vec::new();
        for (z, child) in snapshot.children.iter().enumerate() {
            let (frame, child_ends) = layout_child(snapshot, child, presentation, z as i32, cursor, &mut self.resolver)?;
            frames.push(frame);
            ends.extend(child_ends);
        }
        Ok((frames, ends))
    }
    /// What laying SNAPSHOT out reports, presenting nothing and leaving the
    /// last painted frame, which a cursor-only repaint reuses, in place.
    pub fn lay_out(&mut self, snapshot: &FrameSnapshot) -> Result<Vec<WindowEnd>, String> {
        layout_frame_inner(snapshot, 1, None, &mut self.resolver).map(|(_, ends)| ends)
    }
    /// The host layout hook: INPUT's geometry as painting would lay it out.
    pub fn measure(&mut self, input: &MeasureInput) -> Result<Measured, String> {
        measure_inner(input, &mut self.resolver)
    }
    /// `frame_metrics`, from this engine's fonts.
    pub fn frame_metrics(&mut self, face: &Face) -> Result<(u32, u32), String> {
        let frame = self.layout(&metrics_snapshot(face), 1)?;
        Ok((frame.char_height.ceil() as u32, frame.char_width.ceil() as u32))
    }
    /// The files this engine's answers came from (`FontResolver::font_inputs`).
    pub fn font_inputs(&self) -> Vec<String> {
        self.resolver.font_inputs()
    }
    pub fn layout(&mut self, snapshot: &WindowSnapshot, presentation: u64) -> Result<FrameGlyphBuffer, String> {
        let mut frame = new_frame(snapshot.width, snapshot.height, presentation)?;
        frame.background = color(snapshot.faces.first().ok_or("missing face")?.background);
        append_window(snapshot, &mut frame, &mut self.resolver, 0.0, 0.0, None, &NoFont::default())?;
        Ok(frame)
    }
}

/// A fresh `FrameGlyphBuffer::layout_identity`: every new layout gets
/// one, and a frame reused from the last layout keeps it.
fn next_layout_identity() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Stateless convenience for fixtures; interactive hosts retain a LayoutEngine.
/// One publication for every VM-produced leaf. Commit ends only after paint.
pub fn layout_frame(snapshot: &FrameSnapshot, presentation: u64) -> Result<(FrameGlyphBuffer, Vec<WindowEnd>), String> {
    LayoutEngine::new()?.layout_frame(snapshot, presentation, None)
}

/// Publish point using the same shaped advances and row metrics as text.
/// Selection and color are evaluator-owned policy; this pass owns geometry.
pub fn layout_frame_with_cursors(snapshot: &FrameSnapshot, presentation: u64, selected: u64, cursor_color: Color) -> Result<(FrameGlyphBuffer, Vec<WindowEnd>), String> {
    LayoutEngine::new()?.layout_frame(snapshot, presentation, Some((selected, cursor_color)))
}

/// No cursor in a window of SNAPSHOT whose cursor `internal-show-cursor'
/// turned off.
fn hide_cursors_off(frame: &mut FrameGlyphBuffer, snapshot: &FrameSnapshot) {
    let off: Vec<i64> = snapshot.windows.iter().filter(|w| w.snapshot.cursor_off).map(|w| w.snapshot.id as i64).collect();
    frame.cursor_blinked_off = frame.window_cursors.iter().any(|c| c.active && off.contains(&c.window_id.get()));
    frame.window_cursors.retain(|c| !off.contains(&c.window_id.get()));
}

/// SNAPSHOT laid out with every cursor its windows own, on or off.
fn layout_frame_inner(snapshot: &FrameSnapshot, presentation: u64, cursor: Option<(u64, Color)>, resolver: &mut FontResolver) -> Result<(FrameGlyphBuffer, Vec<WindowEnd>), String> {
    resolver.set_selection_state(snapshot.font_selection);
    resolver.set_alternative_state(snapshot.font_alternatives.clone());
    resolver.set_fontsets(snapshot.fontsets.clone());
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
        let owns = |&(id, _): &(u64, Color)| placed.snapshot.mode_line.is_some() || id == placed.snapshot.id;
        ends.push(append_window(&placed.snapshot, &mut frame, resolver, placed.x as f32, placed.y as f32, cursor.filter(owns).map(|(id, color)| (id == placed.snapshot.id, color)), &snapshot.no_font)?);
    }
    if let Some(tab_bar) = &snapshot.tab_bar { paint_tab_bar(&mut frame, tab_bar, snapshot.width as f32, resolver, &snapshot.no_font)?; }
    if let Some(bar) = &snapshot.status_bar { paint_status_bar(&mut frame, bar, snapshot.chrome.focus, resolver, &snapshot.no_font)?; }
    frame.panes = panes(snapshot);
    Ok((frame, ends))
}

/// SNAPSHOT's windows as panes (VM2-PANE-CHROME): each window's box (its
/// text area, margins and fringes, not its dividers) but the
/// mini-window's, with the frame's corners and hairline, the selected
/// window's edge in the focus colour and the others dimmed; none in GNU's
/// look.
fn panes(snapshot: &FrameSnapshot) -> Vec<romacs_display_protocol::Pane> {
    let chrome = &snapshot.chrome;
    if !chrome.drawn() { return vec![]; }
    snapshot.windows.iter().filter(|placed| placed.snapshot.id != chrome.minibuffer).map(|placed| {
        let (left, right) = placed.snapshot.chrome.insets();
        let selected = placed.snapshot.id == chrome.selected;
        let edge = if selected { chrome.focus.or(chrome.border) } else { chrome.border };
        romacs_display_protocol::Pane {
            bounds: Rect::new((placed.x - left) as f32, placed.y as f32, (left + placed.snapshot.width + right) as f32, placed.snapshot.height as f32),
            radius: chrome.radius as f32,
            edge: edge.map(color),
            dim: if selected { 0.0 } else { chrome.dim },
        }
    }).collect()
}

/// The frame's tab bar row across its top (xdisp.c display_tab_bar): FACE's
/// background, then each character in its face, in faces published above
/// the windows'. A click finds its character by these glyphs, which are
/// the tab bar's only ones and in its text's order.
fn paint_tab_bar(frame: &mut FrameGlyphBuffer, tab_bar: &TabBarSnapshot, width: f32, resolver: &mut FontResolver, no_font: &NoFont) -> Result<(), String> {
    let text = &tab_bar.text;
    validate_runs(text, tab_bar.faces.len())?;
    let face = tab_bar.face.0 as usize;
    if face >= tab_bar.faces.len() || text.utf8.chars().any(char::is_control) { return Err("invalid tab bar".into()); }
    let face_base = frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(0);
    let face_text = face_text(tab_bar.faces.len(), &[(text, face)]);
    let (metrics, glyphless) = realize_faces(&tab_bar.faces, &face_text, |index| index == face, frame, resolver, face_base, Default::default(), no_font)?;
    let bounds = Rect::new(0.0, 0.0, width, tab_bar.height as f32);
    frame.add_background(bounds.x, bounds.y, bounds.width, bounds.height, color(tab_bar.faces[face].background));
    frame.set_draw_context(DisplayWindowId::new(0), GlyphRowRole::TabBar, Some(bounds));
    let boxed = |index: usize| tab_bar.faces[index].decor.boxed.is_some();
    let (mut x, mut box_open) = (0.0, false);
    for (byte, ch) in text.utf8.char_indices() {
        let index = face_at(text, byte, face);
        let id = FaceId::new(face_base + index as u32);
        let (ascent, height, cell, _) = metrics[index];
        let advance = advance(frame, id, ch, cell);
        let background = color(tab_bar.faces[index].background);
        if index != face { frame.add_background(x, 0.0, advance, bounds.height, background); }
        let at = match glyphless.get(index, ch) {
            Some(shown) => {
                let cell = glyphless.paint(frame, index, shown, x, 0.0, (ascent, height), id, true, 0.0)?;
                // Its blank character is there for a click, no cell of a run.
                if boxed(index) { set_glyph_box_edges(frame, cell + 1, edges(false, false)); }
                cell
            }
            None => {
                frame.add_char(ch, x, 0.0, advance, height, ascent, false);
                *frame.glyphs.last_mut().and_then(|g| g.face_id_mut()).ok_or("missing tab bar face")? = id;
                frame.glyphs.len() - 1
            }
        };
        // Boxed characters side by side are one box run (IT_APPLY_FACE_BOX).
        let next = byte + ch.len_utf8();
        let ends = next >= text.utf8.len() || !boxed(face_at(text, next, face));
        if boxed(index) { set_glyph_box_edges(frame, at, edges(!box_open, ends)); }
        box_open = boxed(index) && !ends;
        x += advance;
    }
    Ok(())
}

/// The tab bar's characters as FRAME paints them: each one's left edge
/// and width, in its text's order.
pub fn tab_bar_extents(frame: &FrameGlyphBuffer) -> Vec<(f32, f32)> {
    frame.glyphs.iter().filter_map(|glyph| match glyph {
        romacs_display_protocol::FrameGlyph::Char { row_role: GlyphRowRole::TabBar, x, width, .. } => Some((*x, *width)),
        _ => None,
    }).collect()
}

/// Whether CH is one of Romacs's icons (U+FE000-U+FE0FF): one column wide
/// in any face (VM2-MODE-LINE).
#[inline]
fn is_icon(ch: char) -> bool {
    ('\u{FE000}'..='\u{FE0FF}').contains(&ch)
}

/// Paint the status bar BAR (VM2-STATUS-BAR), a marked band whose
/// accent block is FOCUS where its window holds the input focus, else its
/// face's foreground dimmed.
fn paint_status_bar(frame: &mut FrameGlyphBuffer, bar: &StatusBarSnapshot, focus: Option<[u8; 3]>, resolver: &mut FontResolver, no_font: &NoFont) -> Result<(), String> {
    let face = bar.face.0 as usize;
    if face >= bar.faces.len() { return Err("invalid status bar".into()); }
    let (foreground, background) = (bar.faces[face].foreground, bar.faces[face].background);
    let dimmed = std::array::from_fn(|i| ((u16::from(foreground[i]) + u16::from(background[i])) / 2) as u8);
    let accent = if bar.focused { focus.unwrap_or(foreground) } else { dimmed };
    let bounds = Rect::new(bar.x as f32, bar.y as f32, bar.width as f32, bar.height as f32);
    let band = Marked { text: &bar.text, faces: &bar.faces, face, marks: &bar.marks, bounds, accent: Some(accent) };
    let places = paint_marked(frame, &band, (DisplayWindowId::new(0), GlyphRowRole::StatusBar), resolver, no_font)?;
    frame.status_bar = Some(bounds);
    frame.status_bar_places = places.into_iter().map(|place| place.map(|(x, w)| (bounds.x + x, w))).collect();
    Ok(())
}

/// A band whose painter reads its marks (`BandMarks'): the status bar, or
/// a window's mode line in Romacs's format. Its text starts right of the
/// accent block, drawn in ACCENT when it has one.
struct Marked<'a> {
    text: &'a TextSlice,
    faces: &'a [Face],
    face: usize,
    marks: &'a romacs_layout_input::BandMarks,
    bounds: Rect,
    accent: Option<[u8; 3]>,
}

/// Paint BAND for the window and row role OWNER: its face's background,
/// the accent block, each segment's pill, its characters in its faces on
/// that background (an icon in place of the character it marks, a pill's
/// text in the pill's foreground), the part after its `right-align'
/// marker flush with the right edge (`BandMarks::places'). Where each
/// character went, from the band's left edge.
fn paint_marked(frame: &mut FrameGlyphBuffer, band: &Marked, owner: (DisplayWindowId, GlyphRowRole), resolver: &mut FontResolver, no_font: &NoFont) -> Result<Vec<Option<(f32, f32)>>, String> {
    let (text, face, bounds) = (band.text, band.face, band.bounds);
    validate_runs(text, band.faces.len())?;
    if face >= band.faces.len() || text.utf8.chars().any(char::is_control) { return Err("invalid marked band".into()); }
    let background = band.faces[face].background;
    let faces: Vec<Face> = band.faces.iter().map(|f| Face { background, ..f.clone() }).collect();
    let face_base = frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(0);
    let mut face_text = face_text(faces.len(), &[(text, face)]);
    for &(byte, icon) in &band.marks.icons { face_text[face_at(text, byte, face)].push(icon); }
    let (metrics, glyphless) = realize_faces(&faces, &face_text, |index| index == face, frame, resolver, face_base, Default::default(), no_font)?;
    // Each character once, in order: its byte, what is drawn (an icon in
    // place of the character it marks), its face and advance.
    let mut icons = band.marks.icons.iter().peekable();
    let chars: Vec<(usize, char, usize, f32)> = text.utf8.char_indices().map(|(byte, ch)| {
        while icons.peek().is_some_and(|(at, _)| *at < byte) { icons.next(); }
        let shown = icons.peek().filter(|(at, _)| *at == byte).map_or(ch, |&&(_, icon)| icon);
        let index = face_at(text, byte, face);
        (byte, shown, index, advance(frame, FaceId::new(face_base + index as u32), shown, metrics[index].2))
    }).collect();
    let advances: Vec<f32> = chars.iter().map(|c| c.3).collect();
    let places = band.marks.places(text, bounds.width, STATUS_BAR_PAD, &advances);
    frame.add_background(bounds.x, bounds.y, bounds.width, bounds.height, color(background));
    // The band's own overline runs across it, as GNU's does across a mode line.
    if let Some(line) = band.faces[face].decor.overline {
        frame.add_background(bounds.x, bounds.y, bounds.width, 1.0, color(line.color.unwrap_or(band.faces[face].foreground)));
    }
    if let Some(accent) = band.accent { frame.add_background(bounds.x, bounds.y, STATUS_BAR_ACCENT, bounds.height, color(accent)); }
    // Each pill over what its segment shows, a little inside the band; its
    // text in the pill's own foreground where it has one.
    let mut pill_text: Vec<(std::ops::Range<usize>, [u8; 3])> = vec![];
    for segment in &band.marks.segments {
        let Some(pill) = segment.pill else { continue };
        let first = chars.partition_point(|c| c.0 < segment.bytes.start);
        let end = chars.partition_point(|c| c.0 < segment.bytes.end);
        let shown: Vec<(f32, f32)> = places[first..end].iter().filter_map(|place| *place).collect();
        let (Some(&(start, _)), Some(&(last, width))) = (shown.first(), shown.last()) else { continue };
        let inset = (bounds.height * 0.1).round();
        frame.rounded_fills.push(romacs_display_protocol::RoundedFill {
            bounds: Rect::new(bounds.x + start, bounds.y + inset, last + width - start, bounds.height - 2.0 * inset),
            radius: pill.radius as f32, color: color(pill.background),
        });
        if let Some(text) = pill.foreground { pill_text.push((first..end, text)); }
    }
    // A line folded in from another window (a `context' segment, with its
    // separator) on its own face's background, `mode-line-inactive''s.
    for segment in band.marks.segments.iter().filter(|s| s.name == "context") {
        let first = chars.partition_point(|c| c.0 < segment.bytes.start);
        let end = chars.partition_point(|c| c.0 < segment.bytes.end);
        let shown: Vec<(f32, f32)> = places[first..end].iter().filter_map(|place| *place).collect();
        let (Some(&(start, _)), Some(&(last, width))) = (shown.first(), shown.last()) else { continue };
        let own = band.faces[face_at(text, segment.bytes.start, face)].background;
        if own != background {
            frame.add_background(bounds.x + start, bounds.y, last + width - start, bounds.height, color(own));
        }
    }
    // A pill's text colour: copies of the faces it uses, in that foreground.
    // Above every face realized for the band, its glyphless ones too.
    let mut next = frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(face_base).max(glyphless.end);
    let mut recoloured: std::collections::HashMap<(usize, [u8; 3]), FaceId> = std::collections::HashMap::new();
    let mut ids: Vec<FaceId> = chars.iter().map(|c| FaceId::new(face_base + c.2 as u32)).collect();
    for (range, foreground) in &pill_text {
        for i in range.clone() {
            let key = (chars[i].2, *foreground);
            let id = match recoloured.get(&key) {
                Some(&id) => id,
                None => {
                    let Some(mut copied) = frame.faces.get(&ids[i]).cloned() else { continue };
                    let id = FaceId::new(next);
                    next += 1;
                    copied.id = id;
                    copied.foreground = color(*foreground);
                    frame.faces.insert(id, copied);
                    recoloured.insert(key, id);
                    id
                }
            };
            // The recoloured face draws this character as its source does:
            // only the characters it draws are bound.
            let shown = chars[i].1;
            if let Some(glyph) = frame.char_fonts.get(&ids[i]).and_then(|fonts| fonts.get(&shown)).cloned() {
                frame.char_fonts.entry(id).or_default().insert(shown, glyph);
            }
            ids[i] = id;
        }
    }
    frame.set_draw_context(owner.0, owner.1, Some(bounds));
    let [_, centre_marker, _, right_marker, _] = band.marks.zones(text);
    // A boxed band is one box run, as a mode line is: stretches of its face
    // fill the room between the characters and lie under those whose faces
    // have no box (a mode line's faces merge over its own, so they inherit
    // it), and every cell of a row with a box is the band's height on one
    // baseline (the tallest ascent, a box's top included), so the box's
    // lines run straight.
    let band_box = faces[face].decor.boxed;
    let boxes = band_box.is_some() || chars.iter().any(|c| faces[c.2].decor.boxed.is_some());
    let row_ascent = chars.iter().map(|c| metrics[c.2].0).fold(metrics[face].0, f32::max);
    let band_id = FaceId::new(face_base + face as u32);
    let mut run: Vec<(usize, f32, f32, Option<romacs_layout_input::BoxLine>)> = vec![];
    let fill = |frame: &mut FrameGlyphBuffer, run: &mut Vec<_>, from: f32, to: f32, under: [u8; 3]| {
        if band_box.is_some() && to > from {
            frame.add_stretch(from, bounds.y, to - from, bounds.height, color(under), band_id, false);
            run.push((frame.glyphs.len() - 1, from, to - from, band_box));
        }
    };
    // The context segments, swept with the characters in byte order.
    let mut contexts: Vec<&std::ops::Range<usize>> = band.marks.segments.iter().filter(|s| band_box.is_some() && s.name == "context").map(|s| &s.bytes).collect();
    contexts.sort_by_key(|bytes| bytes.start);
    let mut contexts = contexts.into_iter().peekable();
    let mut filled = bounds.x + band.accent.map_or(0.0, |_| STATUS_BAR_ACCENT);
    for (i, (&(byte, shown, index, _), place)) in chars.iter().zip(&places).enumerate() {
        let Some((x, width)) = *place else { continue };
        if centre_marker.contains(&i) || right_marker.contains(&i) { continue; }
        let (ascent, height) = if boxes { (row_ascent, bounds.height) } else { (metrics[index].0, metrics[index].1) };
        let left = bounds.x + x;
        fill(frame, &mut run, filled, left, background);
        let own = faces[index].decor.boxed;
        if own.is_none() && band_box.is_some() {
            while contexts.next_if(|bytes| bytes.end <= byte).is_some() {}
            let context = contexts.peek().is_some_and(|bytes| bytes.contains(&byte));
            fill(frame, &mut run, left, left + width, if context { band.faces[index].background } else { background });
        }
        let at = match glyphless.get(index, shown) {
            Some(glyph) => {
                let at = glyphless.paint(frame, index, glyph, left, bounds.y, (ascent, height), ids[i], true, 0.0)?;
                // Its blank character is no cell of the run.
                if boxes && matches!(frame.glyphs.get(at + 1), Some(romacs_display_protocol::FrameGlyph::Char { .. })) { frame.glyphs.remove(at + 1); }
                at
            }
            None => {
                frame.add_char(shown, left, bounds.y, width, height, ascent, false);
                *frame.glyphs.last_mut().and_then(|g| g.face_id_mut()).ok_or("missing marked band face")? = ids[i];
                frame.glyphs.len() - 1
            }
        };
        if own.is_some() || band_box.is_none() { run.push((at, left, width, own)); }
        filled = filled.max(left + width);
    }
    fill(frame, &mut run, filled, bounds.x + bounds.width, background);
    // Ends of a run: a neighbour painted against it that shares its box goes on.
    let joins = |a: Option<&(usize, f32, f32, Option<romacs_layout_input::BoxLine>)>, b: &(usize, f32, f32, Option<romacs_layout_input::BoxLine>)|
        a.is_some_and(|a| a.3 == b.3 && (a.1 + a.2 - b.1).abs() < 0.01);
    for k in 0..run.len() {
        if run[k].3.is_none() { continue; }
        let open_left = joins(k.checked_sub(1).and_then(|p| run.get(p)), &run[k]);
        let open_right = run.get(k + 1).is_some_and(|next| joins(Some(&run[k]), next));
        set_glyph_box_edges(frame, run[k].0, edges(!open_left, !open_right));
    }
    Ok(places)
}

/// Where each character of BAR is as FRAME paints it, in its text's
/// order: its left edge in the frame and its width, or None where the bar
/// clipped it; and the indices of its markers' characters, which span the
/// room before the zone each starts and run nothing when clicked.
pub fn status_bar_extents(frame: &FrameGlyphBuffer, bar: &StatusBarSnapshot) -> (Vec<Option<(f32, f32)>>, Vec<usize>) {
    let [_, centre_marker, _, right_marker, _] = bar.marks.zones(&bar.text);
    (frame.status_bar_places.clone(), centre_marker.chain(right_marker).collect())
}

/// CHILD of SNAPSHOT as a frame of its own: its windows inside its border,
/// on its background, placed over the root frame at stacking Z. A child
/// window has the cursor only when it is the selected window.
fn layout_child(snapshot: &FrameSnapshot, child: &ChildFrameSnapshot, presentation: u64, z: i32, cursor: Option<(u64, Color)>, resolver: &mut FontResolver) -> Result<(FrameGlyphBuffer, Vec<WindowEnd>), String> {
    if child.id == 1 || child.windows.is_empty() { return Err("invalid child frame".into()); }
    let frame_snapshot = FrameSnapshot {
        font_selection: snapshot.font_selection, font_alternatives: snapshot.font_alternatives.clone(), fontsets: snapshot.fontsets.clone(),
        width: child.width, height: child.height, windows: child.windows.clone(), background_alpha: child.background_alpha, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None,
        no_font: snapshot.no_font.clone(),
    };
    let cursor = cursor.filter(|(id, _)| child.windows.iter().any(|w| w.snapshot.id == *id));
    let (mut frame, ends) = layout_frame_inner(&frame_snapshot, presentation, cursor, resolver)?;
    frame.background = color(child.background);
    frame.background_alpha = child.background_alpha;
    frame.border_width = child.border as f32;
    frame.border_color = color(child.border_color);
    frame.undecorated = true;
    frame.no_accept_focus = true;
    frame.frame_placement = PresentedFramePlacement::new(
        DisplayFrameId::new(child.id), frame.presentation_id, Some(DisplayFrameId::new(1)),
        ParentFrameRect::new(child.x as f32, child.y as f32, frame.width, frame.height).map_err(|e| e.to_string())?, z,
    );
    hide_cursors_off(&mut frame, &frame_snapshot);
    frame.layer = layer_motion(child, &ends, &snapshot.chrome.style);
    // A rounded layer is light, not chrome: its border is padding, unpainted,
    // and a hairline parts its stacked windows (a picker's input from its list).
    if child.radius > 0 {
        frame.border_width = 0.0;
        separate_windows(child, &mut frame);
    }
    Ok((frame, ends))
}

/// How CHILD animates and looks, with its selection band where ENDS laid
/// its rows.
fn layer_motion(child: &ChildFrameSnapshot, ends: &[WindowEnd], style: &romacs_layout_input::Style) -> romacs_display_protocol::Layer {
    use romacs_display_protocol::{LayerAnimation, LayerEdge, Layer};
    Layer {
        animation: match child.animation {
            romacs_layout_input::LayerAnimation::Off => LayerAnimation::Off,
            romacs_layout_input::LayerAnimation::Reduced => LayerAnimation::Reduced,
            romacs_layout_input::LayerAnimation::Full => LayerAnimation::Full,
        },
        edge: match child.edge {
            romacs_layout_input::LayerEdge::Bottom => LayerEdge::Bottom,
            romacs_layout_input::LayerEdge::Top => LayerEdge::Top,
        },
        selection: child.selection.and_then(|row| selection_band(child, ends.first()?, row, style)),
        radius: child.radius as f32,
        glow: child.glow.map(color),
        glow_light: style.glow,
        shadow: child.shadow.then_some(style.shadow),
        outline: child.outline.map(color),
    }
}

/// A 1px line across CHILD above each of its windows but the topmost, in
/// its background lifted 15% toward the default foreground.
fn separate_windows(child: &ChildFrameSnapshot, frame: &mut FrameGlyphBuffer) {
    let Some(top) = child.windows.iter().map(|w| w.y).min() else { return };
    let Some(fg) = child.windows.first().and_then(|w| w.snapshot.faces.first()).map(|f| f.foreground) else { return };
    let line = color(std::array::from_fn(|i| (f32::from(child.background[i]) * 0.85 + f32::from(fg[i]) * 0.15).round() as u8));
    for placed in child.windows.iter().filter(|w| w.y > top && w.snapshot.height > 0) {
        frame.add_background(0.0, placed.y as f32 - 1.0, child.width as f32, 1.0, line);
    }
}

/// The band row ROW of CHILD's first window shows its selection in, laid
/// out as END: a picker's selected row (Zed's inset `ListItem`, ideas and
/// values only): the window's width and up to 4px of the padding either
/// side, the row's height, STYLE's corners, in the background of the face
/// at the row's start; none past the rows shown or under a header line.
fn selection_band(child: &ChildFrameSnapshot, end: &WindowEnd, row: u32, style: &romacs_layout_input::Style) -> Option<romacs_display_protocol::SelectionBand> {
    let placed = child.windows.first()?;
    let window = &placed.snapshot;
    let row = end.rows.get(row as usize).filter(|_| window.header_line.is_none())?;
    let face = window.text.face_runs.iter().find(|run| run.bytes.contains(&row.start)).map_or(0, |run| run.face.0 as usize);
    let inset = (placed.x as f32).min(4.0);
    Some(romacs_display_protocol::SelectionBand {
        x: placed.x as f32 - inset, y: placed.y as f32 + row.y, width: window.width as f32 + 2.0 * inset, height: row.height,
        radius: style.band_radius,
        color: color(window.faces.get(face)?.background), base: color(window.faces.first()?.background),
    })
}

fn new_frame(width: u32, height: u32, presentation: u64) -> Result<FrameGlyphBuffer, String> {
    if width == 0 || height == 0 { return Err("invalid frame extent or presentation".into()); }
    let mut frame = FrameGlyphBuffer::with_size(width as f32, height as f32);
    stamp(&mut frame, presentation)?;
    Ok(frame)
}

/// FRAME as presentation PRESENTATION.
fn stamp(frame: &mut FrameGlyphBuffer, presentation: u64) -> Result<(), String> {
    if presentation == 0 { return Err("invalid frame extent or presentation".into()); }
    frame.presentation_id = PresentationId::new(presentation);
    frame.frame_placement = PresentedFramePlacement::new(
        DisplayFrameId::new(1), frame.presentation_id, None,
        ParentFrameRect::new(0.0, 0.0, frame.width, frame.height).map_err(|e| e.to_string())?, 0,
    );
    Ok(())
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

/// LTR rows, continued or truncated as GNU does, with the window's
/// fringes and their indicators; no cursor or policy evaluation.
/// Byte positions are relative to the owned UTF-8 text, not VM heap references.
pub fn layout(snapshot: &WindowSnapshot, presentation: u64) -> Result<FrameGlyphBuffer, String> {
    let mut frame = new_frame(snapshot.width, snapshot.height, presentation)?;
    frame.background = color(snapshot.faces.first().ok_or("missing face")?.background);
    let mut resolver = FontResolver::new().map_err(|e| e.to_string())?;
    append_window(snapshot, &mut frame, &mut resolver, 0.0, 0.0, None, &NoFont::default())?;
    Ok(frame)
}

/// Paint SNAPSHOT's window at OX, OY, and report its rows (`WindowEnd`).
fn append_window(snapshot: &WindowSnapshot, frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver, ox: f32, oy: f32, cursor: Option<(bool, Color)>, no_font: &NoFont) -> Result<WindowEnd, String> {
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
        return Ok(WindowEnd { id: snapshot.id, end: snapshot.start_byte, visible_end: snapshot.start_byte, body_height: 0.0, rows: vec![], point: None });
    }
    validate_runs(&snapshot.text, snapshot.faces.len())?;
    if snapshot.stretches.windows(2).any(|w| w[0].byte >= w[1].byte)
        || snapshot.stretches.iter().any(|s| text.get(s.byte..s.byte + 1) != Some(" ")) {
        return Err("stretches out of order or not on a placeholder space".into());
    }
    if snapshot.images.windows(2).any(|w| w[0].byte >= w[1].byte)
        || snapshot.images.iter().any(|i| text.get(i.byte..i.byte + 1) != Some(" ")) {
        return Err("images out of order or not on a placeholder space".into());
    }
    let mut stretches = snapshot.stretches.iter().peekable();
    let mut images = snapshot.images.iter().peekable();
    // The row's images so far: their glyphs' indexes and ascents.
    let mut row_images: Vec<(usize, f32)> = vec![];
    let bands = [(&snapshot.header_line, snapshot.header_line_face), (&snapshot.mode_line, snapshot.mode_line_face)];
    for (band, face) in bands {
        let Some(band) = band else { continue };
        validate_runs(band, snapshot.faces.len())?;
        if face.0 as usize >= snapshot.faces.len() || band.utf8.chars().any(char::is_control) {
            return Err("invalid mode-line face or control character".into());
        }
    }
    // Above every face id an earlier window took, its band's copies included.
    let face_base = frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(0);
    let mut slices = vec![(&snapshot.text, 0)];
    slices.extend(bands.iter().filter_map(|(band, face)| Some(((*band).as_ref()?, face.0 as usize))));
    let mut face_text = face_text(snapshot.faces.len(), &slices);
    if let Some(numbers) = &snapshot.line_numbers {
        if numbers.face.0 as usize >= snapshot.faces.len() || numbers.current.0 as usize >= snapshot.faces.len() {
            return Err("invalid line-number face".into());
        }
        for face in [numbers.face, numbers.current] { face_text[face.0 as usize].push_str("0123456789 "); }
    }
    // A display table's truncation and continuation glyphs show in the
    // default face, from whichever font has them.
    if let Some(default) = face_text.first_mut() { default.extend(snapshot.chrome.line_glyphs.iter().flatten()); }
    // A band's own face sets its height even where no character uses it,
    // as the faces of the fringes' bitmaps set their colours.
    let band_faces: Vec<usize> = bands.iter().filter(|(band, _)| band.is_some()).map(|(_, face)| face.0 as usize)
        .chain(fringe::bitmap_faces(&snapshot.chrome))
        .chain(snapshot.text.extends.iter().map(|e| e.face.0 as usize)).collect();
    if snapshot.text.extends.iter().any(|e| e.face.0 as usize >= snapshot.faces.len())
        || snapshot.text.extends.windows(2).any(|w| w[0].byte >= w[1].byte) {
        return Err("invalid extend face".into());
    }
    let (metrics, glyphless) = realize_faces(&snapshot.faces, &face_text, |index| band_faces.contains(&index), frame, resolver, face_base, snapshot.underline_rules, no_font)?;
    if face_base == 0 { frame.char_height = metrics[0].1; frame.char_width = metrics[0].2; }
    // A band is as tall as its tallest face.
    let band_height = |(band, face): (&Option<TextSlice>, FaceHandle)| band.as_ref().map_or(0.0, |band| {
        std::iter::once(face.0 as usize)
            .chain(band.face_runs.iter().map(|run| run.face.0 as usize))
            .map(|index| metrics[index].1)
            .fold(0.0_f32, f32::max)
    });
    let (header_height, mode_height) = (band_height(bands[0]), band_height(bands[1]));
    let body_height = (snapshot.height as f32 - header_height - mode_height).max(0.0);
    // The text area starts below the header line: OY is its origin from here.
    let (window_top, oy) = (oy, oy + header_height);
    frame.set_draw_context(
        DisplayWindowId::new(i64::try_from(snapshot.id).map_err(|e| e.to_string())?),
        GlyphRowRole::Text,
        Some(Rect::new(ox, oy, snapshot.width as f32, body_height)),
    );
    // GNU: a row ends where its next glyph would pass the text area,
    // less the continuation (`\\') or truncation (`$') glyph it shows there
    // on a window without a right fringe, whose indicators show the break
    // instead (display_line, move_it_in_display_line_to, init_iterator).
    // Rows are laid out from the start of the window start's line, so a
    // window starting on a continuation line breaks its rows, and stops
    // its TABs, as from the line start.
    // A horizontally scrolled window truncates its lines (init_iterator).
    let truncate = snapshot.wrap == LineWrap::Truncate || !snapshot.hscroll.is_zero();
    // A display table's glyph, where it sets one, for GNU's `$' and `\\'.
    let [truncation, continuation] = snapshot.chrome.line_glyphs;
    let special = if truncate { truncation.unwrap_or('$') } else { continuation.unwrap_or('\\') };
    let default_id = FaceId::new(face_base);
    let glyphs_mark = snapshot.chrome.right_fringe == 0;
    let last_x = snapshot.width as f32 - if glyphs_mark { advance(frame, default_id, special, metrics[0].2) } else { 0.0 };
    let line_start = text[..snapshot.start_byte].rfind('\n').map_or(0, |at| at + 1);
    // Each line shows from its first visible x (GNU's first_visible_x):
    // point's line from the hscroll of point's row. Its rows are truncated.
    let point_line = text[..snapshot.point_byte].rfind('\n').map_or(0, |at| at + 1);
    let shift_of = |line: usize| if line == point_line { snapshot.hscroll.point_row } else { snapshot.hscroll.rows } as f32;
    // A scrolled row's left truncation glyph, without a left fringe
    // (insert_left_trunc_glyphs): it hides the glyphs it covers.
    let left_glyph = truncation.unwrap_or('$');
    let dollar = if snapshot.chrome.left_fringe == 0 { advance(frame, default_id, left_glyph, metrics[0].2) } else { 0.0 };
    let left_mark = |shift: f32| if shift > 0.0 { dollar } else { 0.0 };
    // Line numbers take each row's first glyphs, its line's label or as
    // many blanks, in cells of their face.
    // (x, the label when the row starts its line, whether point's line).
    let gutter = |byte: usize| -> (f32, Option<&str>, bool) {
        let Some(numbers) = &snapshot.line_numbers else { return (0.0, None, false) };
        let cell = metrics[numbers.face.0 as usize].2;
        let at = numbers.labels.partition_point(|(b, ..)| *b <= byte);
        match at.checked_sub(1).map(|i| &numbers.labels[i]) {
            Some((b, label, current)) if *b == byte => (label.len() as f32 * cell, Some(label.as_str()), *current),
            Some((_, label, current)) => (label.len() as f32 * cell, None, *current),
            None => (numbers.columns as f32 * cell, None, false),
        }
    };
    // A `word-wrap' window's rows break where GNU's word wrap breaks them.
    let word = snapshot.wrap == LineWrap::Word && !truncate;
    // Text that shows no box skips the box runs' work (`box_step').
    let boxes = shows_box(&snapshot.faces, &snapshot.text);
    let mut breaks = WordBreaks { row: line_start, continued: 0.0, next: None, done: !word, boxes, box_open: false };
    let mut x0 = gutter(line_start).0;
    let mut x = x0;
    let mut shift = shift_of(line_start);
    // Where point is (`PointPlace'), and where the row being built was
    // truncated on the right, unscrolled.
    let mut place: Option<PointPlace> = None;
    let mut cut = 0.0f32;
    // The cursor's x on a row scrolled by SHIFT for a position at
    // unscrolled X: on the truncation glyph past the row's last position,
    // or on its first glyph when scrolled out on the left
    // (set_cursor_from_row's glyph_after and glyph_before).
    let cursor_x = |x: f32, x0: f32, shift: f32, hidden: bool, cut: f32| {
        if hidden { cut - shift } else if x < x0 + shift { x0 } else { x - shift }
    };
    let mut y = -(snapshot.vscroll_px as f32);
    let mut row_height = metrics[0].1;
    let mut end_byte = snapshot.start_byte;
    let mut point_rect = None;
    // The width of the rows of this line above this one, and the glyphs
    // on this row; past the truncation glyph, the rest of the line is hidden.
    let (mut continued, mut hpos, mut hidden) = (0.0f32, 0usize, false);
    // Whether the glyph before is in a box run; the tallest box top on the
    // row being built, and where its glyphs start (`settle_row').
    let mut box_open = false;
    let (mut row_box_top, mut row_glyphs) = (0.0f32, frame.glyphs.len());
    let mut drawing = line_start == snapshot.start_byte;
    // The first byte of the row being built, and the end of the rows
    // above it that show whole: a row is fully visible when it lies
    // within the text area (GNU's cursor_row_fully_visible_p).
    let (mut row_start, mut visible_end) = (snapshot.start_byte, snapshot.start_byte);
    let fully_visible = |y: f32, height: f32| y >= 0.0 && y + height <= body_height;
    // Each drawn row as it ends; the last, as the text ends. Beside each,
    // how it breaks, which the fringes indicate; whether the row being
    // built continues a line a break or the window start split.
    let mut rows: Vec<Row> = vec![];
    let mut marks: Vec<RowMarks> = vec![];
    let mut continuation = line_start != snapshot.start_byte;
    // The end of the composed sequence being laid out, and its glyph's x
    // and advance.
    let (mut inside, mut composed) = (0, (0.0f32, 0.0f32));
    for (byte, ch) in text[line_start..].char_indices().map(|(at, ch)| (line_start + at, ch)) {
        // The window start begins a row: at a line start, or where the row
        // before it breaks, which the character there decides below, so
        // that row's width counts toward `continued' first.
        let mut starting = !drawing && byte >= snapshot.start_byte;
        let top = -(snapshot.vscroll_px as f32);
        if starting && (x == x0 || ch == '\n') {
            (drawing, starting) = (true, false);
            x0 = gutter(byte).0;
            (x, y, row_height, hpos) = (x0, top, metrics[0].1, 0);
        }
        if drawing && y >= body_height { break; }
        if drawing || starting { end_byte = byte + ch.len_utf8(); }
        let at_point = (drawing || starting) && byte == snapshot.point_byte;
        if at_point && ch == '\n' {
            point_rect = Some((cursor_x(x, x0, shift, hidden, cut), y, metrics[0].2, row_height, metrics[0].0));
            place = Some(PointPlace { x: cursor_x(x, x0, shift, hidden, cut), line_x: x + continued, gutter: x0,
                truncated_left: shift > 0.0, truncated_right: hidden, at_eol: true });
        }
        if ch == '\n' {
            if drawing {
                // xdisp.c extend_face_to_end_of_line: the row paints past
                // its text, to the text area's edge, in its extend face.
                if let Some(extend) = snapshot.text.extends.binary_search_by_key(&(byte as u32), |e| e.byte).ok().map(|i| snapshot.text.extends[i]) {
                    let index = extend.face.0 as usize;
                    let from = (x - shift).max(x0);
                    let to = snapshot.width as f32;
                    if !hidden && body_height > 0.0 && y < body_height && y + row_height > 0.0 && to > from {
                        frame.add_stretch(ox + from, oy + y, to - from, row_height, color(snapshot.faces[index].background),
                                          FaceId::new(face_base + index as u32), false);
                    }
                }
                if visible_end == row_start && fully_visible(y, row_height) { visible_end = byte + 1; }
                row_height = settle_row(frame, row_glyphs, oy + y, row_height, row_box_top > 0.0, &row_images, &metrics, face_base);
                (row_box_top, row_glyphs) = (0.0, frame.glyphs.len());
                row_images.clear();
                rows.push(Row { start: row_start, y, height: row_height });
                marks.push(RowMarks { continued: false, continuation, truncated: hidden, truncated_left: shift > 0.0 });
                if shift > 0.0 && left_mark(shift) > 0.0 && y < body_height && y + metrics[0].1 > 0.0 {
                    glyph(frame, left_glyph, ox + x0, oy + y, metrics[0], default_id)?;
                }
                if let Some(place) = place.as_mut().filter(|_| row_start <= snapshot.point_byte && snapshot.point_byte <= byte) {
                    place.truncated_right = hidden;
                }
                row_start = byte + 1;
            }
            continuation = false;
            box_open = false;
            x0 = gutter(byte + 1).0;
            x = x0;
            shift = shift_of(byte + 1);
            y += row_height;
            row_height = metrics[0].1;
            (continued, hpos, hidden) = (0.0, 0, false);
            continue;
        }
        while stretches.next_if(|s| s.byte < byte).is_some() {}
        let stretch = stretches.next_if(|s| s.byte == byte).map(|s| s.width);
        while images.next_if(|i| i.byte < byte).is_some() {}
        let image = images.next_if(|i| i.byte == byte).copied();
        // Past the truncation, only point's x on its line still counts.
        let measuring = hidden && drawing && row_start >= point_line && byte < snapshot.point_byte;
        if hidden && !measuring {
            if at_point {
                point_rect = Some((cut - shift, y, metrics[0].2, row_height, metrics[0].0));
                place = Some(PointPlace { x: cut - shift, line_x: x + continued, gutter: x0, truncated_left: shift > 0.0, truncated_right: true, at_eol: false });
            }
            continue;
        }
        let index = face_at(&snapshot.text, byte, 0);
        let id = FaceId::new(face_base + index as u32);
        let (ascent, height, cell, space) = metrics[index];
        let shown = match image {
            Some(image) => Shown::Image(image),
            None => shown_at(&snapshot.text, byte, ch, stretch, &mut inside, frame, id, cell),
        };
        // An image's slot is its own height (produce_image_glyph).
        let (ascent, height) = match &shown { Shown::Image(i) => (i.ascent as f32, (i.ascent + i.descent) as f32), _ => (ascent, height) };
        // X on the row and from the line's start (`continued' before it);
        // a TAB stops as from after the line number (x_produce_glyphs).
        let width = |frame: &FrameGlyphBuffer, x: f32, line_x: f32, x0: f32|
            shown_advance(frame, &shown, id, cell, space, snapshot.tab_width, x, line_x, x0, (!truncate).then_some(last_x));
        // A box run (xdisp.c start_of_box_run_p, end_of_box_run_p): its
        // text's first and last glyphs own its left and right edges; edges
        // outside the text are spacers of their width, the run's advance.
        let step = if boxes { box_step(&snapshot.faces, &snapshot.text, (0, index), byte, ch, shows_glyphs(&shown), box_open, false) } else { BoxStep::NONE };
        // The run's state past this character, on every path below; a row
        // break alone opens no new run (the iterator's box state carries).
        let opening = !box_open;
        box_open = step.open;
        let (lead, trail) = (step.lead, step.trail);
        let mut advance_px = width(frame, x, x + continued, x0) + lead + trail;
        if measuring { x += advance_px; continue; }
        // A word-wrapped row breaks where `WordBreaks' finds; another, at
        // the glyph that would pass the text area.
        // No row breaks inside a composed sequence.
        let breaking = if word { breaks.at(byte, snapshot, frame, &metrics, face_base, last_x, &|byte| gutter(byte).0) }
                       else { !truncate && hpos > 0 && x + advance_px > last_x && !matches!(shown, Shown::Inside) };
        if breaking {
            if glyphs_mark && drawing && y < body_height && y + metrics[0].1 > 0.0 { glyph(frame, special, ox + x, oy + y, metrics[0], default_id)?; }
            if drawing {
                if visible_end == row_start && fully_visible(y, row_height) { visible_end = byte; }
                row_height = settle_row(frame, row_glyphs, oy + y, row_height, row_box_top > 0.0, &row_images, &metrics, face_base);
                (row_box_top, row_glyphs) = (0.0, frame.glyphs.len());
                row_images.clear();
                rows.push(Row { start: row_start, y, height: row_height });
                marks.push(RowMarks { continued: true, continuation, truncated: false, truncated_left: false });
                row_start = byte;
            }
            continuation = true;
            // A TAB that does not fit continues from the text area's edge.
            continued += if matches!(shown, Shown::Tab) { last_x } else { x };
            x0 = gutter(byte).0;
            (x, y, row_height, hpos) = (x0, y + row_height, metrics[0].1, 0);
            if starting { (drawing, starting, y) = (true, false, top); }
            // Continued past the last row, this character is not shown.
            if drawing && y >= body_height { end_byte = byte; }
            advance_px = width(frame, x0, x0 + continued, x0) + lead + trail;
            if at_point { point_rect = Some((x, y, metrics[0].2, row_height, metrics[0].0)); }
        }
        if starting {
            // A start inside a row (the motion refuses one): the row
            // breaks there.
            continuation = true;
            continued += x;
            x0 = gutter(byte).0;
            (drawing, x, y, row_height, hpos) = (true, x0, top, metrics[0].1, 0);
            advance_px = width(frame, x0, x0 + continued, x0) + lead + trail;
            if at_point { point_rect = Some((x, y, metrics[0].2, row_height, metrics[0].0)); }
        }
        row_height = row_height.max(height);
        let visible = drawing && body_height > 0.0 && y < body_height && y + height > 0.0 && x - shift < snapshot.width as f32;
        if truncate && x + advance_px > last_x + shift && !matches!(shown, Shown::Inside) {
            if visible && glyphs_mark { glyph(frame, special, ox + x - shift, oy + y, metrics[0], default_id)?; }
            (hidden, cut) = (true, x);
            if at_point {
                point_rect = Some((x - shift, y, metrics[0].2, row_height, ascent));
                place = Some(PointPlace { x: x - shift, line_x: x + continued, gutter: x0, truncated_left: shift > 0.0, truncated_right: true, at_eol: false });
            }
            // The rest of the line still counts toward point's x on it.
            x += advance_px;
            continue;
        }
        // Scrolled out on the left, or under the left truncation glyph; a
        // glyph partly out shows its rest, but none overlaps a line number.
        let left_edge = x0 + shift + left_mark(shift);
        let visible = visible && x + advance_px > x0 + shift && (x >= left_edge || (snapshot.line_numbers.is_none() && left_mark(shift) == 0.0));
        let ox = ox - shift;
        use romacs_display_protocol::BoxVerticalEdges as Edges;
        // A stretch in a box run owns the run's edges at its ends; its
        // advance has the sides in it.
        let stretch_edges = |frame: &mut FrameGlyphBuffer| if step.boxed { set_box_edges(frame, edges(opening, step.ends)); };
        let cursor = match &shown {
            Shown::Tab => {
                if visible {
                    frame.add_stretch(ox + x, oy + y, advance_px, height, color(snapshot.faces[index].background), id, false);
                    stretch_edges(frame);
                }
                // GNU's cursor on a stretch is no wider than a column
                // (`x-stretch-cursor' nil).
                advance_px.min(metrics[0].2)
            }
            // A stretch that is not positive is no glyph (nglyphs 0).
            Shown::Stretch(_) => {
                if visible && advance_px > 0.0 {
                    frame.add_stretch(ox + x, oy + y, advance_px, height, color(snapshot.faces[index].background), id, false);
                    stretch_edges(frame);
                }
                advance_px.clamp(0.0, metrics[0].2)
            }
            // The slot in the face's background, the image's slice in it.
            Shown::Image(image) => {
                if visible {
                    frame.add_stretch(ox + x, oy + y, advance_px, height, color(snapshot.faces[index].background), id, false);
                    if image.image != 0 {
                        let (sx, sy, sw, sh) = image.source;
                        let (fw, fh) = (image.full.0.max(1) as f32, image.full.1.max(1) as f32);
                        let source = romacs_display_protocol::image::ImageSourceRect::new(sx as f32 / fw, sy as f32 / fh, sw as f32 / fw, sh as f32 / fh)
                            .unwrap_or(romacs_display_protocol::image::ImageSourceRect::FULL);
                        frame.add_image(romacs_display_protocol::ImageId::new(image.image),
                                        ox + x + image.body_x as f32, oy + y + image.body_y as f32, sw as f32, sh as f32);
                        // The face's background under the image is its body's; the
                        // slot's stretch paints the margins, where a relief goes.
                        if let Some(romacs_display_protocol::FrameGlyph::Image { source_rect, slot_rect, face_id, .. }) = frame.glyphs.last_mut() {
                            *source_rect = source;
                            *slot_rect = romacs_display_protocol::Rect::new(ox + x, oy + y, advance_px, height);
                            *face_id = id;
                        }
                    }
                    // The background slot and the image in it move together,
                    // listed in glyph order.
                    let slot_at = frame.glyphs.len() - 1 - usize::from(image.image != 0);
                    row_images.extend((slot_at..frame.glyphs.len()).map(|at| (at, ascent)));
                    // An image that failed to load: a rectangle of its size in
                    // the face's foreground (x_draw_image_foreground).
                    if image.image == 0 {
                        let (_, _, sw, sh) = image.source;
                        let (bx, by, bw, bh) = (ox + x + image.body_x as f32, oy + y + image.body_y as f32, sw as f32, sh as f32);
                        let ink = color(snapshot.faces[index].foreground);
                        for (sx, sy, w, h) in [(bx, by, bw, 1.0), (bx, by + bh - 1.0, bw, 1.0), (bx, by, 1.0, bh), (bx + bw - 1.0, by, 1.0, bh)] {
                            frame.add_stretch(sx, sy, w, h, ink, id, false);
                            row_images.push((frame.glyphs.len() - 1, ascent));
                        }
                    }
                    let base = image.relief_background.unwrap_or_else(|| {
                        let [r, g, b] = snapshot.faces[index].background;
                        u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
                    });
                    for (sx, sy, sw, sh, pixel) in relief_strips(&image, ox + x, oy + y, base) {
                        frame.add_stretch(sx, sy, sw, sh, Color::from_pixel(pixel), id, false);
                        row_images.push((frame.glyphs.len() - 1, ascent));
                    }
                }
                advance_px
            }
            // Outside a box run, a character's glyphs are only themselves.
            Shown::Glyphs(glyphs) if !step.boxed => {
                let (mut at, mut first) = (x, None);
                for c in glyphs.chars() {
                    let width = advance(frame, id, c, cell);
                    if visible && at - shift < snapshot.width as f32 {
                        match glyphless.get(index, c) {
                            Some(shown) => { glyphless.paint(frame, index, shown, ox + at, oy + y, (ascent, height), id, false, 0.0)?; }
                            None => glyph_of(frame, c, ox + at, oy + y, width, (ascent, height), id)?,
                        }
                    }
                    first.get_or_insert(width);
                    at += width;
                }
                first.unwrap_or(cell)
            }
            Shown::Glyphs(glyphs) => {
                // A side outside the text: the left a spacer before it, the
                // right the last glyph's own (its cell, the cursor's width).
                if visible && lead > 0.0 {
                    frame.add_stretch(ox + x, oy + y, lead, height, color(snapshot.faces[index].background), id, false);
                    set_box_edges(frame, Edges::Left);
                }
                let (mut at, mut first) = (x + lead, None);
                let count = glyphs.chars().count();
                for (n, c) in glyphs.chars().enumerate() {
                    let last = n + 1 == count;
                    let width = advance(frame, id, c, cell) + if last { trail } else { 0.0 };
                    if visible && at - shift < snapshot.width as f32 {
                        // A glyphless character's cell in the run is its background.
                        let at_cell = match glyphless.get(index, c) {
                            Some(shown) => glyphless.paint(frame, index, shown, ox + at, oy + y, (ascent, height), id, false, width - shown.width as f32)?,
                            None => { glyph_of(frame, c, ox + at, oy + y, width, (ascent, height), id)?; frame.glyphs.len() - 1 }
                        };
                        set_glyph_box_edges(frame, at_cell, edges(step.side == 0.0 && n == 0 && opening, last && step.ends));
                    }
                    first.get_or_insert(width);
                    at += width;
                }
                if visible { row_box_top = row_box_top.max(step.top); }
                // The cursor covers the glyph with its box's sides.
                first.unwrap_or(cell) + lead
            }
            Shown::Composed { end, advance } => {
                // A sequence's glyph takes a box run's sides as a character's does.
                if visible && x - shift < snapshot.width as f32 {
                    if lead > 0.0 {
                        frame.add_stretch(ox + x, oy + y, lead, height, color(snapshot.faces[index].background), id, false);
                        set_box_edges(frame, Edges::Left);
                    }
                    publish_selected_glyph(frame, resolver, id, &text[byte..*end]);
                    frame.add_composed_char(&text[byte..*end], ch, ox + x + lead, oy + y, *advance + trail, height, ascent, false);
                    *frame.glyphs.last_mut().and_then(|g| g.face_id_mut()).ok_or("missing character face")? = id;
                    if step.boxed { set_box_edges(frame, edges(step.side == 0.0 && opening, step.ends)); }
                    row_box_top = row_box_top.max(step.top);
                }
                composed = (x, *advance + lead);
                *advance + lead
            }
            Shown::Inside => 0.0,
        };
        if at_point {
            // Point inside a composed sequence shows on its glyph.
            let (x, cursor) = if matches!(shown, Shown::Inside) { composed } else { (x, cursor) };
            let at = cursor_x(x, x0, shift, false, cut);
            point_rect = Some((at, y, if at == x - shift { cursor } else { metrics[0].2 }, row_height, ascent));
            place = Some(PointPlace { x: at, line_x: x + continued, gutter: x0, truncated_left: shift > 0.0, truncated_right: false, at_eol: false });
        }
        if !matches!(shown, Shown::Stretch(_)) || advance_px > 0.0 { hpos += 1; }
        x += advance_px;
    }
    if snapshot.point_byte == text.len() && end_byte == text.len() {
        let at = cursor_x(x, x0, shift, hidden, cut);
        point_rect = Some((at, y, metrics[0].2, row_height, metrics[0].0));
        // An empty last row shows no character: GNU truncates it on neither side.
        let empty = row_start == text.len();
        place = Some(PointPlace { x: at, line_x: x + continued, gutter: x0, truncated_left: shift > 0.0 && !empty, truncated_right: hidden, at_eol: true });
    }
    if let Some(place) = place.as_mut().filter(|_| row_start <= snapshot.point_byte) { place.truncated_right |= hidden; }
    // The last row holds the text's end position too, when layout reached it.
    if end_byte == text.len() && visible_end == row_start && fully_visible(y, row_height) {
        visible_end = text.len() + 1;
    }
    // The row ending at point-max extends its face too (the default's,
    // which paints only its lines there).
    if end_byte == text.len() && !hidden && y < body_height && y + row_height > 0.0 &&
        let Some(extend) = snapshot.text.extends.last().filter(|e| e.byte as usize == text.len()) {
        let index = extend.face.0 as usize;
        let from = (x - shift).max(x0);
        let to = snapshot.width as f32;
        if to > from {
            frame.add_stretch(ox + from, oy + y, to - from, row_height, color(snapshot.faces[index].background),
                              FaceId::new(face_base + index as u32), false);
        }
    }
    if end_byte == text.len() && y < body_height {
        let scrolled = shift > 0.0 && row_start < text.len();
        row_height = settle_row(frame, row_glyphs, oy + y, row_height, row_box_top > 0.0, &row_images, &metrics, face_base);
        rows.push(Row { start: row_start, y, height: row_height });
        marks.push(RowMarks { continued: false, continuation, truncated: hidden, truncated_left: scrolled });
        if scrolled && left_mark(shift) > 0.0 && y + metrics[0].1 > 0.0 { glyph(frame, left_glyph, ox + x0, oy + y, metrics[0], default_id)?; }
    }
    fringe::paint(frame, snapshot, &rows, &marks, fringe::Area { ox, oy, body_height, line: metrics[0].1, text_len: text.len(), reached_end: end_byte == text.len(), face_base })?;
    // A row starting a numbered line shows its label; any other, blanks,
    // in the face of line numbers.
    if let Some(numbers) = &snapshot.line_numbers {
        for row in rows.iter().filter(|r| r.y < body_height && r.y + r.height > 0.0) {
            // A continuation row of point's line keeps its face, in blanks.
            let (x0, label, current) = gutter(row.start);
            let index = if current { numbers.current.0 } else { numbers.face.0 } as usize;
            let id = FaceId::new(face_base + index as u32);
            let (ascent, height, cell, _) = metrics[index];
            let blanks = " ".repeat((x0 / cell).round() as usize);
            let mut at = 0.0;
            for c in label.unwrap_or(blanks.as_str()).chars() {
                let width = advance(frame, id, c, cell);
                if at < snapshot.width as f32 { glyph_of(frame, c, ox + at, oy + row.y, width, (ascent, height), id)?; }
                at += width;
            }
        }
    }
    if let (Some((active, color)), Some((x, y, width, height, ascent))) = (cursor, point_rect) {
        // No cursor may bleed into the mode line or another window.
        // With a right fringe, a cursor after a row that fills the text
        // area shows in the fringe (`overflow-newline-into-fringe').
        let right = snapshot.width as f32 + snapshot.chrome.right_fringe as f32;
        if x >= 0.0 && x < right && y >= 0.0 && y + height <= body_height {
            frame.add_cursor(DisplayWindowId::new(snapshot.id as i64), ox + x, oy + y,
                width.min(right - x), height,
                if active { romacs_display_protocol::CursorStyle::FilledBox } else { romacs_display_protocol::CursorStyle::Hollow }, color);
            let published = frame.window_cursors.last_mut().unwrap();
            published.active = active;
            published.ascent = ascent;
        }
    }
    // Each band's faces are the next table after the window's, so the
    // next window's faces start after them.
    // The header and mode lines span the whole window, margins and
    // fringes included.
    let (left, right) = snapshot.chrome.insets();
    let (band_x, band_width) = (ox - left as f32, (left + snapshot.width + right) as f32);
    let mut copy = 0;
    if let Some(band) = &snapshot.header_line {
        copy += 1;
        let bounds = Rect::new(band_x, window_top, band_width, header_height);
        let stretches = (&snapshot.band_stretches.header_line[..], left as f32);
        paint_band(frame, snapshot, band, snapshot.header_line_face, bounds, stretches, GlyphRowRole::HeaderLine, (face_base, copy), (&metrics, &glyphless))?;
    }
    if let Some(band) = &snapshot.mode_line {
        copy += 1;
        let bounds = Rect::new(band_x, oy + body_height, band_width, snapshot.height as f32 - header_height - body_height);
        let stretches = (&snapshot.band_stretches.mode_line[..], left as f32);
        paint_band(frame, snapshot, band, snapshot.mode_line_face, bounds, stretches, GlyphRowRole::ModeLine, (face_base, copy), (&metrics, &glyphless))?;
    }
    Ok(WindowEnd { id: snapshot.id, end: end_byte, visible_end, body_height, rows, point: place })
}

/// Paint BAND, a header or mode line, in BOUNDS: its FACE's background,
/// then its characters in copies of the window's faces on that background,
/// published from FACE_BASE plus COPY face tables on. A character STRETCHES
/// names is that stretch, blank (produce_stretch_glyph), its `:align-to'
/// counted from the text area's left edge, TEXT_LEFT into the band, as
/// GNU's full-width band row measures it (calc_pixel_width_or_height).
#[allow(clippy::too_many_arguments)]
fn paint_band(frame: &mut FrameGlyphBuffer, snapshot: &WindowSnapshot, band: &TextSlice, face: FaceHandle, bounds: Rect,
              (stretches, text_left): (&[romacs_layout_input::Stretch], f32),
              role: GlyphRowRole, (face_base, copy): (u32, u32), (metrics, glyphless): (&[Metrics], &Glyphless)) -> Result<(), String> {
    let default = face.0 as usize;
    let background = color(snapshot.faces[default].background);
    frame.add_background(bounds.x, bounds.y, bounds.width, bounds.height, background);
    frame.set_draw_context(DisplayWindowId::new(i64::try_from(snapshot.id).map_err(|e| e.to_string())?), role, Some(bounds));
    let offset = u32::try_from(snapshot.faces.len()).map_err(|e| e.to_string())?.checked_mul(copy).ok_or("too many faces")?;
    for index in 0..snapshot.faces.len() {
        let source = FaceId::new(face_base + index as u32);
        let target = FaceId::new(face_base.checked_add(offset).and_then(|v| v.checked_add(index as u32)).ok_or("too many faces")?);
        let Some(copied) = frame.faces.get(&source) else { continue };
        let mut copied = copied.clone();
        copied.id = target;
        copied.background = background;
        frame.faces.insert(target, copied);
        if let Some(chars) = frame.char_fonts.get(&source).cloned() {
            frame.char_fonts.insert(target, chars);
        }
    }
    use romacs_display_protocol::BoxVerticalEdges as Edges;
    // The band's face fills its rest (display_mode_line), so a boxed band
    // face's run goes on into the fill, whose end closes it.
    let fill_boxed = snapshot.faces[default].decor.boxed.is_some();
    let (mut x, mut box_open, mut boxed_top) = (0.0, false, false);
    let first_glyph = frame.glyphs.len();
    // The last glyph painted while a run stays open past it.
    let mut open_glyph = None;
    let mut stretches = stretches.iter().peekable();
    for (byte, ch) in band.utf8.char_indices() {
        let index = face_at(band, byte, default);
        let id = FaceId::new(face_base + offset + index as u32);
        let (ascent, height, cell, _) = metrics[index];
        while stretches.peek().is_some_and(|s| s.byte < byte) { stretches.next(); }
        let stretch = stretches.next_if(|s| s.byte == byte);
        // A stretch keeps its width in a box run (IT_APPLY_FACE_BOX).
        let step = box_step(&snapshot.faces, band, (default, index), byte, ch, stretch.is_none(), box_open, fill_boxed);
        let opening = !box_open;
        box_open = step.open;
        if let Some(stretch) = stretch {
            let width = match stretch.width {
                StretchWidth::AlignTo(target) => StretchWidth::AlignTo(target + text_left as i64),
                width => width,
            };
            let advance = stretch_advance(width, f64::from(x), f64::from(x), None) as f32;
            // Blank, but a face's lines and box run across it (produce_stretch_glyph).
            if x < bounds.width && advance > 0.0 && snapshot.faces[index].decor != Default::default() {
                frame.add_stretch(bounds.x + x, bounds.y, advance.min(bounds.width - x), bounds.height, background, id, false);
                let at = frame.glyphs.len() - 1;
                if step.boxed {
                    // No side spacer pads a stretch: it owns the left edge.
                    set_glyph_box_edges(frame, at, edges(opening, step.ends));
                    boxed_top |= step.top > 0.0;
                }
                open_glyph = step.open.then_some(at);
            }
            x += advance;
            continue;
        }
        let advance = advance(frame, id, ch, cell);
        if x < bounds.width {
            if step.lead > 0.0 {
                frame.add_stretch(bounds.x + x, bounds.y, step.lead, height, background, id, false);
                set_box_edges(frame, Edges::Left);
            }
            // The right side outside the text is the glyph's own; a
            // glyphless character's cell in the run is its background.
            let at_cell = match glyphless.get(index, ch) {
                Some(shown) => glyphless.paint(frame, index, shown, bounds.x + x + step.lead, bounds.y, (ascent, height), id, false, step.trail)?,
                None => {
                    frame.add_char(ch, bounds.x + x + step.lead, bounds.y, advance + step.trail, height, ascent, false);
                    *frame.glyphs.last_mut().and_then(|g| g.face_id_mut()).ok_or("missing mode-line face")? = id;
                    frame.glyphs.len() - 1
                }
            };
            if step.boxed {
                set_glyph_box_edges(frame, at_cell, edges(step.side == 0.0 && opening, step.ends));
                boxed_top |= step.top > 0.0;
            }
            open_glyph = step.open.then_some(at_cell);
        }
        x += advance + step.lead + step.trail;
    }
    // display_mode_line: the rest of the band is the band's face, as the
    // spaces GNU fills it with; past the background, only its lines show.
    if x < bounds.width && snapshot.faces[default].decor != Default::default() {
        frame.add_stretch(bounds.x + x, bounds.y, bounds.width - x, bounds.height, background,
                          FaceId::new(face_base + offset + default as u32), false);
        if fill_boxed { set_box_edges(frame, if box_open { Edges::Right } else { Edges::Both }); }
    } else if let Some(at) = open_glyph {
        // Text that fills or passes the band: its last glyph shown closes
        // the run at the band's edge.
        let closed = match frame.glyphs[at].box_vertical_edges() { Some(Edges::Left | Edges::Both) => Edges::Both, _ => Edges::Right };
        set_glyph_box_edges(frame, at, closed);
        // Its cell ends at the band's edge, so the edge shows
        // (pgtkterm.c clamps the box's right to the area's).
        if let romacs_display_protocol::FrameGlyph::Char { x, width, .. } | romacs_display_protocol::FrameGlyph::Stretch { x, width, .. } = &mut frame.glyphs[at] {
            *width = width.min(bounds.x + bounds.width - *x).max(0.0);
        }
    }
    // One baseline for the band, a box's top included.
    if boxed_top {
        settle_row(frame, first_glyph, bounds.y, bounds.height, true, &[], &metrics, face_base + offset);
    }
    Ok(())
}

/// A box run at one character (xdisp.c start_of_box_run_p,
/// end_of_box_run_p), a TAB's or a stretch's too: whether its face (DEFAULT
/// where TEXT names none; INDEX, the character's) has a box; for a character PADS (text, not a TAB
/// or a stretch, whose widths GNU keeps: IT_APPLY_FACE_BOX), the side spacers before it (a run
/// starts where the glyph before, OPEN or not, was not in one) and after
/// it (the run ends where the next character's face has none, at a
/// newline, or at the text's end unless TAIL continues it); the side and
/// top widths outside the text; whether the run goes on past it.
struct BoxStep { boxed: bool, lead: f32, trail: f32, side: f32, top: f32, ends: bool, open: bool }

impl BoxStep {
    /// A character outside any box run.
    const NONE: BoxStep = BoxStep { boxed: false, lead: 0.0, trail: 0.0, side: 0.0, top: 0.0, ends: false, open: false };
}

#[allow(clippy::too_many_arguments)]
fn box_step(faces: &[Face], text: &TextSlice, (default, index): (usize, usize), byte: usize, ch: char, pads: bool, open: bool, tail: bool) -> BoxStep {
    let boxed = faces[index].decor.boxed;
    if boxed.is_none() { return BoxStep::NONE; }
    let next = byte + ch.len_utf8();
    let utf8 = &text.utf8;
    let ends = boxed.is_some() && if next >= utf8.len() { !tail } else {
        utf8[next..].starts_with('\n') || faces[face_at(text, next, default)].decor.boxed.is_none()
    };
    let side = boxed.map_or(0.0, |b| f32::from(b.vertical.max(0)));
    BoxStep {
        boxed: boxed.is_some(),
        lead: if open || !pads { 0.0 } else { side },
        trail: if ends && pads { side } else { 0.0 },
        side,
        top: boxed.map_or(0.0, |b| f32::from(b.horizontal.max(0))),
        ends,
        open: boxed.is_some() && !ends,
    }
}

/// Whether any character of TEXT shows in a face of FACES with a box: the
/// default face's, or a run's.  A window's face table is the frame's, so
/// most of its boxed faces show nowhere in its text.
fn shows_box(faces: &[Face], text: &TextSlice) -> bool {
    let boxed = |index: usize| faces.get(index).is_some_and(|face| face.decor.boxed.is_some());
    boxed(0) || text.face_runs.iter().any(|run| boxed(run.face.0 as usize))
}

/// Settle the row at TOP, HEIGHT tall, whose glyphs start at FROM, on one
/// baseline, as produce_glyphs's row ascent is the largest of its glyphs'
/// (a boxed glyph's grown by its box's top): each character and each
/// stretch shorter than the row moves down by that ascent less its own
/// (the ascent METRICS[face - FACE_BASE] gives a stretch); the row's new height.  Only a
/// row with a box (BOXED) is settled.
fn settle_row(frame: &mut FrameGlyphBuffer, from: usize, top: f32, height: f32, boxed: bool, images: &[(usize, f32)],
              metrics: &[(f32, f32, f32, f32)], face_base: u32) -> f32 {
    use romacs_display_protocol::FrameGlyph;
    if !boxed && images.is_empty() { return height; }
    // An image's slot and the image in it rise to its own ascent.
    // IMAGES are in glyph order.
    let ascent_of = |at: usize, glyph: &FrameGlyph| match images.binary_search_by_key(&at, |&(i, _)| i) {
        Ok(k) => Some(images[k].1),
        Err(_) => match glyph {
            FrameGlyph::Char { ascent, .. } => Some(*ascent),
            FrameGlyph::Stretch { height: h, face_id, .. } if *h < height =>
                metrics.get((face_id.get() as usize).wrapping_sub(face_base as usize)).map(|m| m.0),
            _ => None,
        },
    };
    let row_ascent = frame.glyphs[from..].iter().enumerate().filter_map(|(k, g)| ascent_of(from + k, g)).fold(0.0f32, f32::max);
    let mut bottom = height;
    for k in from..frame.glyphs.len() {
        let Some(own) = ascent_of(k, &frame.glyphs[k]) else { continue };
        let shift = row_ascent - own;
        match &mut frame.glyphs[k] {
            FrameGlyph::Char { y, baseline, height: h, .. } => { *y += shift; *baseline += shift; bottom = bottom.max(*y + *h - top); }
            FrameGlyph::Stretch { y, height: h, .. } => { *y += shift; bottom = bottom.max(*y + *h - top); }
            FrameGlyph::Image { y, slot_rect, box_rect, .. } => {
                *y += shift;
                slot_rect.y += shift;
                box_rect.y += shift;
                bottom = bottom.max(slot_rect.y + slot_rect.height - top);
            }
            _ => {}
        }
    }
    bottom
}

/// pgtk_draw_image_relief's strips around IMAGE's slice, its slot at X,
/// Y: |relief| thick just inside its margin, the top and left lighter
/// and the bottom and right darker than BASE when raised, the other way
/// when sunken (pgtk_setup_relief_colors), each side only where the slice
/// has the image's edge.  As rectangles: GNU's mitred corners aside.
fn relief_strips(image: &romacs_layout_input::InlineImage, x: f32, y: f32, base: u32) -> Vec<(f32, f32, f32, f32, u32)> {
    let thick = image.relief.abs() as f32;
    if thick == 0.0 { return vec![]; }
    let (light, dark) = (lighter_color(base, 1.2, 0x8000), lighter_color(base, 0.6, 0x4000));
    let (top_left, bottom_right) = if image.relief > 0 { (light, dark) } else { (dark, light) };
    let (sx, sy, sw, sh) = image.source;
    let (left, top) = (x + image.body_x as f32 - thick, y + image.body_y as f32 - thick);
    let (width, height) = (sw as f32 + 2.0 * thick, sh as f32 + 2.0 * thick);
    let (left_p, top_p) = (sx == 0, sy == 0);
    let (right_p, bot_p) = (sx + sw == image.full.0, sy + sh == image.full.1);
    let mut strips = vec![];
    if left_p { strips.push((left, top, thick, height, top_left)); }
    if right_p { strips.push((left + width - thick, top, thick, height, bottom_right)); }
    // The top stops short of the right side, the bottom of the left.
    if top_p { strips.push((left, top, width - if right_p { thick } else { 0.0 }, thick, top_left)); }
    if bot_p { let from = if left_p { thick } else { 0.0 }; strips.push((left + from, top + height - thick, width - from, thick, bottom_right)); }
    // A relief wider than a pixel has a pixel of the other color outside.
    if left_p && thick > 1.0 { strips.push((left, top, 1.0, height, bottom_right)); }
    if top_p && thick > 1.0 { strips.push((left, top, width, 1.0, bottom_right)); }
    strips
}

/// pgtk_compute_lighter_color: PIXEL (0xRRGGBB) scaled by FACTOR, a dark
/// one moved DELTA (of 0xffff) further, or DELTA added where that changes
/// nothing.
fn lighter_color(pixel: u32, factor: f64, delta: i64) -> u32 {
    let channel = |shift: u32| i64::from((pixel >> shift) & 0xff) * 257;
    let (r, g, b) = (channel(16), channel(8), channel(0));
    let scale = |c: i64| ((factor * c as f64) as i64).min(0xffff);
    let (mut nr, mut ng, mut nb) = (scale(r), scale(g), scale(b));
    let bright = (2 * r + 3 * g + b) / 6;
    if bright < 48000 {
        let dimness = 1.0 - bright as f64 / 48000.0;
        let min_delta = (delta as f64 * dimness * factor / 2.0) as i64;
        let adjust = |c: i64| if factor < 1.0 { (c - min_delta).max(0) } else { (c + min_delta).min(0xffff) };
        (nr, ng, nb) = (adjust(nr), adjust(ng), adjust(nb));
    }
    let pack = |r: i64, g: i64, b: i64| ((r >> 8) << 16 | (g >> 8) << 8 | (b >> 8)) as u32;
    let new = pack(nr, ng, nb);
    if new != pixel { return new; }
    pack((delta + r).min(0xffff), (delta + g).min(0xffff), (delta + b).min(0xffff))
}

/// A box run glyph's edges: the LEFT and the RIGHT, either or neither.
fn edges(left: bool, right: bool) -> romacs_display_protocol::BoxVerticalEdges {
    use romacs_display_protocol::BoxVerticalEdges as Edges;
    match (left, right) { (true, true) => Edges::Both, (true, false) => Edges::Left, (false, true) => Edges::Right, (false, false) => Edges::Neither }
}

/// Which of its box run's vertical edges the glyph AT owns.
fn set_glyph_box_edges(frame: &mut FrameGlyphBuffer, at: usize, edges: romacs_display_protocol::BoxVerticalEdges) {
    use romacs_display_protocol::FrameGlyph;
    if let Some(FrameGlyph::Char { box_vertical_edges, .. } | FrameGlyph::Stretch { box_vertical_edges, .. }) = frame.glyphs.get_mut(at) {
        *box_vertical_edges = edges;
    }
}

/// Which of its box run's vertical edges the glyph just added owns.
fn set_box_edges(frame: &mut FrameGlyphBuffer, edges: romacs_display_protocol::BoxVerticalEdges) {
    use romacs_display_protocol::FrameGlyph;
    if let Some(FrameGlyph::Char { box_vertical_edges, .. } | FrameGlyph::Stretch { box_vertical_edges, .. }) = frame.glyphs.last_mut() {
        *box_vertical_edges = edges;
    }
}

/// Draw CH at X, Y in face ID, whose ascent, height and nominal advance
/// METRICS gives.
fn glyph(frame: &mut FrameGlyphBuffer, ch: char, x: f32, y: f32, metrics: (f32, f32, f32, f32), id: FaceId) -> Result<(), String> {
    let (ascent, height, cell, _) = metrics;
    let width = advance(frame, id, ch, cell);
    glyph_of(frame, ch, x, y, width, (ascent, height), id)
}

/// Draw CH, WIDTH wide, at X, Y in face ID of ASCENT and HEIGHT.
fn glyph_of(frame: &mut FrameGlyphBuffer, ch: char, x: f32, y: f32, width: f32, (ascent, height): (f32, f32), id: FaceId) -> Result<(), String> {
    frame.add_char(ch, x, y, width, height, ascent, false);
    *frame.glyphs.last_mut().and_then(|g| g.face_id_mut()).ok_or("missing character face")? = id;
    Ok(())
}

/// How a buffer character other than newline is drawn: a TAB as a
/// stretch, another control character as GNU's escape (`^X`, or `\NNN`
/// for one beyond ASCII), anything else as itself; a placeholder space
/// the snapshot names a stretch, as that stretch.
///
/// An emoji or variation sequence (`romacs_layout_input::cluster`) shows as
/// one composed glyph at its first character, ADVANCE wide, through END;
/// the characters after it are inside it and take no room.
enum Shown { Tab, Stretch(StretchWidth), Image(romacs_layout_input::InlineImage), Glyphs(Glyphs), Composed { end: usize, advance: f32 }, Inside }

/// At most four glyphs, the longest escape (`\NNN`), held without
/// allocating.
#[derive(Clone, Copy)]
struct Glyphs { chars: [char; 4], len: usize }
impl Glyphs {
    fn of(chars: &[char]) -> Self {
        let mut held = ['\0'; 4];
        held[..chars.len()].copy_from_slice(chars);
        Glyphs { chars: held, len: chars.len() }
    }
    fn chars(&self) -> impl Iterator<Item = char> + '_ { self.chars[..self.len].iter().copied() }
}

#[inline]
fn shown(ch: char, raw: bool) -> Shown {
    let digit = |shift: u32| char::from_digit((ch as u32 >> shift) & 7, 8).unwrap_or('0');
    match ch {
        '\t' => Shown::Tab,
        '\u{0}'..='\u{1f}' | '\u{7f}' => Shown::Glyphs(Glyphs::of(&['^', char::from((ch as u8) ^ 0x40)])),
        // GNU's three octal digits: the code's low nine bits; a raw byte's
        // placeholder has the byte's code.
        _ if raw || ch.is_control() => Shown::Glyphs(Glyphs::of(&['\\', digit(6), digit(3), digit(0)])),
        _ => Shown::Glyphs(Glyphs::of(&[ch])),
    }
}

/// What the character CH at BYTE of TEXT shows as, STRETCH the snapshot's
/// stretch there, INSIDE the end of the composed sequence the characters
/// before it began (updated when one begins here), in face ID of nominal
/// advance CELL. A sequence is as wide as the glyph its representative
/// character has (`representative_char_for_cluster').
#[allow(clippy::too_many_arguments)]
#[inline]
fn shown_at(text: &TextSlice, byte: usize, ch: char, stretch: Option<StretchWidth>, inside: &mut usize,
            frame: &FrameGlyphBuffer, id: FaceId, cell: f32) -> Shown {
    if let Some(width) = stretch { return Shown::Stretch(width); }
    // No sequence starts at, or holds, an ASCII character.
    if ch.is_ascii() { return shown(ch, text.is_raw(byte)); }
    sequence_at(text, byte, ch, inside, frame, id, cell)
}

/// `shown_at' for a character beyond ASCII, out of the ASCII text's way.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn sequence_at(text: &TextSlice, byte: usize, ch: char, inside: &mut usize, frame: &FrameGlyphBuffer, id: FaceId, cell: f32) -> Shown {
    if byte < *inside { return Shown::Inside; }
    let raw = text.is_raw(byte);
    if !raw && let Some(end) = romacs_layout_input::cluster::cluster_end(&text.utf8, byte) {
        *inside = end;
        let representative = romacs_font::cluster::representative_char_for_cluster(&text.utf8[byte..end]);
        return Shown::Composed { end, advance: representative.map_or(cell, |c| advance(frame, id, c, cell)) };
    }
    shown(ch, raw)
}

/// Whether SHOWN draws glyphs (a composed sequence's included), which a
/// box run's edges belong to, rather than a stretch.
fn shows_glyphs(shown: &Shown) -> bool {
    matches!(shown, Shown::Glyphs(_) | Shown::Composed { .. } | Shown::Inside)
}

/// Publish the one glyph a sequence of a single character and its
/// selectors (VS-16's `❤️') shows in face ID, when it has one: the
/// character's glyph in the font its representative resolved to (the emoji
/// font for VS-16), so the renderer draws that exact glyph rather than
/// shaping the text in the text font. Other sequences (ZWJ ligatures) are
/// shaped by the renderer.
fn publish_selected_glyph(frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver, id: FaceId, cluster: &str) {
    let mut bases = cluster.chars().filter(|&c| !romacs_layout_input::cluster::is_composition_joiner(c));
    let (Some(base), None) = (bases.next(), bases.next()) else { return };
    if frame.shaped_clusters.get(&id).is_some_and(|published| published.contains_key(cluster)) { return; }
    let Some(representative) = romacs_font::cluster::representative_char_for_cluster(cluster) else { return };
    let Some(font) = frame.char_fonts.get(&id).and_then(|chars| chars.get(&representative))
        .and_then(|binding| frame.fonts.get(&binding.resolved_font_id)) else { return };
    let Some((glyph_id, x_advance)) = resolver.glyph_in_font(font, base) else { return };
    let glyph = romacs_display_protocol::font::ResolvedGlyph { resolved_font_id: font.id, glyph_id, x: 0.0, y: 0.0, x_advance,
        cluster_start: 0, cluster_end: cluster.len() as u32 };
    frame.shaped_clusters.entry(id).or_default().insert(cluster.into(), vec![glyph]);
}

/// The advance of SHOWN in face ID (CELL wide, SPACE its space's width)
/// at row X, line x LINE_X, on a row whose text starts at X0: a TAB to
/// its next stop as from after the line number, a stretch as GNU's (up to
/// LAST_X on a continued row), glyphs by their own advances.
#[allow(clippy::too_many_arguments)]
fn shown_advance(frame: &FrameGlyphBuffer, shown: &Shown, id: FaceId, cell: f32, space: f32, tab_width: u32,
                 x: f32, line_x: f32, x0: f32, last_x: Option<f32>) -> f32 {
    match shown {
        Shown::Tab => tab_advance(f64::from(line_x - x0), f64::from(space), tab_width) as f32,
        Shown::Stretch(width) => stretch_advance(*width, f64::from(x), f64::from(line_x), last_x.map(f64::from)) as f32,
        Shown::Image(image) => image.width as f32,
        Shown::Glyphs(glyphs) => glyphs.chars().map(|glyph| advance(frame, id, glyph, cell)).sum(),
        Shown::Composed { advance, .. } => *advance,
        Shown::Inside => 0.0,
    }
}

/// GNU's word wrap (move_it_in_display_line_to), one row ahead of the
/// layout: the bytes where a `word-wrap' window's rows break, from ROW, a
/// line start. A row breaks when its next glyph would pass LAST_X, at its
/// last wrap point, the first glyph after whitespace (a space or TAB, as
/// `wrap_flips' classes the snapshot's elements), or
/// with none at that glyph; the next row continues the line from there,
/// its TABs stopping as from the line's start (a TAB that does not fit,
/// from the text area's edge). Rows are found only as the layout reaches
/// them, so a long line costs what the window shows.
struct WordBreaks { row: usize, continued: f32, next: Option<usize>, done: bool, boxes: bool, box_open: bool }

impl WordBreaks {
    /// Whether a row breaks at BYTE; BYTEs ascend.
    #[allow(clippy::too_many_arguments)]
    fn at(&mut self, byte: usize, snapshot: &WindowSnapshot, frame: &FrameGlyphBuffer, metrics: &[(f32, f32, f32, f32)],
          face_base: u32, last_x: f32, gutter: &dyn Fn(usize) -> f32) -> bool {
        while !self.done && self.next.is_none_or(|next| next < byte) {
            self.next = self.row_break(snapshot, frame, metrics, face_base, last_x, gutter);
            self.done = self.next.is_none();
        }
        self.next == Some(byte)
    }

    /// The next break after the rows already found, or None at the text's end.
    fn row_break(&mut self, snapshot: &WindowSnapshot, frame: &FrameGlyphBuffer, metrics: &[(f32, f32, f32, f32)],
                 face_base: u32, last_x: f32, gutter: &dyn Fn(usize) -> f32) -> Option<usize> {
        let text = &snapshot.text.utf8;
        let stretch_at = |byte: usize| snapshot.stretches.binary_search_by_key(&byte, |s| s.byte).ok().map(|i| snapshot.stretches[i].width);
        let image_at = |byte: usize| snapshot.images.binary_search_by_key(&byte, |i| i.byte).ok().map(|i| snapshot.images[i]);
        while self.row < text.len() {
            let (row, x0) = (self.row, gutter(self.row));
            let (mut x, mut hpos, mut may_wrap, mut wrap_at) = (x0, 0usize, false, None::<(usize, f32, bool)>);
            let (mut end, mut inside) = (None, 0);
            for (byte, ch) in text[row..].char_indices().map(|(at, ch)| (row + at, ch)) {
                if ch == '\n' { end = Some((byte + 1, None)); self.box_open = false; break; }
                let whitespace = (ch == ' ' || ch == '\t') != snapshot.wrap_flips.binary_search(&(byte as u32)).is_ok();
                // The box state before BYTE, where the row may restart.
                let before = self.box_open;
                if may_wrap && !whitespace { wrap_at = Some((byte, x, before)); }
                may_wrap = whitespace;
                let index = face_at(&snapshot.text, byte, 0);
                let (_, _, cell, space) = metrics[index];
                let id = FaceId::new(face_base + index as u32);
                let shown = match image_at(byte) {
                    Some(image) => Shown::Image(image),
                    None => shown_at(&snapshot.text, byte, ch, stretch_at(byte), &mut inside, frame, id, cell),
                };
                // A box run's sides count as painting counts them.
                let step = if self.boxes { box_step(&snapshot.faces, &snapshot.text, (0, index), byte, ch, shows_glyphs(&shown), self.box_open, false) } else { BoxStep::NONE };
                self.box_open = step.open;
                let advance = shown_advance(frame, &shown, id, cell, space, snapshot.tab_width, x, x + self.continued, x0, Some(last_x))
                    + step.lead + step.trail;
                // No row breaks inside a composed sequence.
                if hpos > 0 && x + advance > last_x && !matches!(shown, Shown::Inside) {
                    let (at, x, open) = wrap_at.unwrap_or((byte, if matches!(shown, Shown::Tab) { last_x } else { x }, before));
                    self.box_open = open;
                    end = Some((at, Some(x)));
                    break;
                }
                if !matches!(shown, Shown::Stretch(_)) || advance > 0.0 { hpos += 1; }
                x += advance;
            }
            match end {
                None => { self.row = text.len(); }
                Some((at, None)) => { (self.row, self.continued) = (at, 0.0); }
                Some((at, Some(x))) => { (self.row, self.continued) = (at, self.continued + x); return Some(at); }
            }
        }
        None
    }
}

/// The face table index painting SLICE's character at BYTE.
fn face_at(slice: &TextSlice, byte: usize, default: usize) -> usize {
    // Runs are ordered and disjoint (`validate_runs`).
    let at = slice.face_runs.partition_point(|run| run.bytes.end <= byte);
    slice.face_runs.get(at).filter(|run| run.bytes.contains(&byte)).map_or(default, |run| run.face.0 as usize)
}

/// Each face's painted characters. Publishing the whole window for every
/// color-only face multiplies Unicode bindings by the face-table size,
/// even when those faces are unused.
fn face_text(faces: usize, slices: &[(&TextSlice, usize)]) -> Vec<String> {
    let mut text = vec![String::new(); faces];
    // The continuation and truncation glyphs, in the default face.
    text[0].push_str("\\$");
    for (slice, default) in slices {
        // A composed sequence's characters are all its first one's face's,
        // which draws it (and needs its representative's font).
        let mut inside = 0;
        for (byte, ch) in slice.utf8.char_indices() {
            if byte < inside { continue; }
            let face = &mut text[face_at(slice, byte, *default)];
            if !ch.is_ascii() && !slice.is_raw(byte) && let Some(end) = romacs_layout_input::cluster::cluster_end(&slice.utf8, byte) {
                face.push_str(&slice.utf8[byte..end]);
                inside = end;
                continue;
            }
            match shown(ch, slice.is_raw(byte)) {
                Shown::Tab | Shown::Stretch(_) | Shown::Image(_) => face.push(' '),
                Shown::Glyphs(glyphs) => face.extend(glyphs.chars()),
                Shown::Composed { .. } | Shown::Inside => face.push(ch),
            }
        }
    }
    text
}

/// Where FACE's underline goes in FONT, and how thick it is, as xterm.c
/// x_draw_glyph_string decides: the font's thickness, else 1; at the
/// descent line (less `:position' pixels) under RULES' at-descent-line or a
/// `:position', else the font's own position under `use_position' when it
/// has one, else half its descent, never above RULES' minimum offset.
fn underline_placement(face: &Face, font: &romacs_display_protocol::font::ResolvedFont, rules: romacs_layout_input::UnderlineRules)
    -> (romacs_display_protocol::face::UnderlinePosition, i32) {
    use romacs_display_protocol::face::UnderlinePosition;
    let thickness = if font.underline.thickness > 0 { font.underline.thickness } else { 1 };
    let pixels_above = face.decor.underline.and_then(|line| line.position);
    if rules.at_descent_line || pixels_above.is_some() {
        return (UnderlinePosition::DescentLine { pixels_above: u32::from(pixels_above.unwrap_or(0)) }, thickness);
    }
    let position = if rules.use_position && font.underline.position >= 0 { font.underline.position }
        else { (font.descent_px.round() as i32 + 1) / 2 };
    (UnderlinePosition::FontMetric { offset_from_baseline: position.max(rules.minimum_offset.min(i32::MAX as u32) as i32) }, thickness)
}

/// Each face's ascent, height, nominal advance and space width, as
/// `realize_face' publishes it from FACE_BASE on, and the glyphless glyphs
/// of the characters no font has (`glyphless'), in faces published above
/// every copy of the table a window's bands make. A face no character uses
/// is not published, except the default and those KEEP accepts; its
/// metrics are unused.
#[allow(clippy::too_many_arguments)]
fn realize_faces(faces: &[Face], face_text: &[String], keep: impl Fn(usize) -> bool, frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver,
                 face_base: u32, rules: romacs_layout_input::UnderlineRules, no_font: &NoFont) -> Result<(Vec<Metrics>, Glyphless), String> {
    let mut metrics = Vec::new();
    let mut glyphless = Glyphless::default();
    let len = u32::try_from(faces.len()).map_err(|e| e.to_string())?;
    // Above a window's band copies (face_base + 1 and 2 tables): each
    // face's digit and box-line faces, then two copies of its digit face.
    let derived = len.checked_mul(3).and_then(|n| n.checked_add(face_base)).ok_or("too many faces")?;
    glyphless.end = len.checked_mul(4).and_then(|n| n.checked_add(derived)).ok_or("too many faces")?;
    let id_at = |base: u32, index: usize| u32::try_from(index).ok().and_then(|i| base.checked_add(i)).map(FaceId::new).ok_or("too many faces");
    for (index, face) in faces.iter().enumerate() {
        if index != 0 && !keep(index) && face_text[index].is_empty() {
            metrics.push((0.0, 0.0, 0.0, 0.0));
            continue;
        }
        let id = id_at(face_base, index)?;
        let realized = realize_face(face, faces.first(), id, &face_text[index], frame, resolver, rules)?;
        metrics.push(realized.metrics);
        if realized.unresolved.is_empty() { continue; }
        // GNU's no-font glyph (produce_glyphless_glyph), its code in
        // `glyphless-char' (the character's face at 0.6 of its height) and
        // its box in the face's foreground.
        let (digits, line) = (id_at(derived, 2 * index)?, id_at(derived, 2 * index + 1)?);
        let spare = [id_at(derived, 2 * faces.len() + 2 * index)?, id_at(derived, 2 * faces.len() + 2 * index + 1)?];
        let small = Face { pixel_size: (face.pixel_size * glyphless::DIGIT_SCALE).round().max(1.0), decor: Default::default(), ..face.clone() };
        let shown: String = realized.unresolved.iter().filter_map(|&ch| glyphless::text_of(no_font, ch)).collect();
        let digit_face = realize_face(&small, None, digits, &shown, frame, resolver, Default::default()).ok();
        let fg = color(face.foreground);
        frame.set_face_with_font(line, fg, Some(fg), &face.family, face.weight, face.italic, face.pixel_size, 0, None, 0, None, 0, None, false);
        let primary = frame.fonts[&realized.primary].clone();
        let digit_font = digit_face.as_ref().map(|d| frame.fonts[&d.primary].clone());
        let base_width = resolver.ink_extents(&primary, "x").map_or(realized.metrics.2, |x| x.width).round() as i32;
        let base = glyphless::Base { ascent: primary.ascent_px.round() as i32, descent: primary.descent_px.round() as i32, width: base_width };
        let shown_in = glyphless::Faces { digits: digit_face.as_ref().map(|d| (digits, d.metrics.2)), line, spare };
        for &ch in &realized.unresolved {
            let mut extents = |text: &str| digit_font.as_ref().and_then(|font| resolver.ink_extents(font, text));
            let glyph = glyphless::layout(no_font, ch, base, &mut extents);
            glyphless.bind(frame, (index, id), &primary, ch, glyph, shown_in);
        }
    }
    Ok((metrics, glyphless))
}

/// A face's ascent, height, nominal advance and space width.
type Metrics = (f32, f32, f32, f32);

/// What `realize_face' made of one face: its metrics, its primary font,
/// and the characters of its text no installed font has a glyph for.
struct Realized {
    metrics: Metrics,
    primary: romacs_display_protocol::font::ResolvedFontId,
    unresolved: Vec<char>,
}

/// Note WHAT once per process: a display-time font failure is degraded,
/// never fatal, and is reported the first time it happens.
fn warn_once(what: String) {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);
    let Ok(mut seen) = SEEN.lock() else { return };
    let seen = seen.get_or_insert_with(Default::default);
    if seen.len() < 256 && seen.insert(what.clone()) { eprintln!("romacs-layout: {what}"); }
}

/// Publish FACE as ID with its font, binding the characters of TEXT, and
/// its metrics: its ascent, height, nominal advance and space width.
/// Nothing here stops the display: a face whose font cannot be had falls
/// back to the DEFAULT face's family and then to `monospace' (GNU falls
/// back to the frame's font), a font without a positive advance to its
/// space or half its size, and a character no font has is left for the
/// caller to show glyphless.
#[allow(clippy::too_many_arguments)]
fn realize_face(face: &Face, default: Option<&Face>, id: FaceId, text: &str, frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver,
                rules: romacs_layout_input::UnderlineRules) -> Result<Realized, String> {
    let valid = |f: &Face| !f.family.is_empty() && f.pixel_size.is_finite() && f.pixel_size > 0.0;
    let pixel_size = if face.pixel_size.is_finite() && face.pixel_size > 0.0 { face.pixel_size }
        else { default.filter(|d| valid(d)).map_or(16.0, |d| d.pixel_size) };
    let asked = FontRequest { pixel_size, ..font_request(face) };
    // The families tried after the face's own, built only when it fails.
    let fallbacks = [default.filter(|d| valid(d)).map(|d| d.family.as_str()), Some("monospace")];
    let line = |line: Option<romacs_layout_input::Line>| line.map_or((0, None), |l| (l.style, l.color.map(color)));
    let ((underline, underline_color), (strike, strike_color), (overline, overline_color)) =
        (line(face.decor.underline), line(face.decor.strike), line(face.decor.overline));
    // Icons are bound from the resolver's one answer for them, not
    // resolved through the face's fallback on every layout.
    let plain: std::borrow::Cow<str> = if text.chars().any(is_icon) { text.chars().filter(|&c| !is_icon(c)).collect::<String>().into() } else { text.into() };
    let mut failure = None;
    let (request, published) = 'found: {
        let mut tried: Vec<&str> = vec![];
        for family in std::iter::once(None).chain(fallbacks.into_iter().flatten().map(Some)) {
            let request = match family {
                None => asked.clone(),
                Some(family) if family == asked.family || tried.contains(&family) => continue,
                Some(family) => { tried.push(family); FontRequest { family: family.to_owned(), ..asked.clone() } }
            };
            if request.family.is_empty() { continue; }
            frame.set_face_with_font(id, color(face.foreground), Some(color(face.background)), &request.family, face.weight, face.italic, pixel_size,
                underline, underline_color, strike, strike_color, overline, overline_color, false);
            match publish_face_text(resolver, frame, id, &request, &plain) {
                Ok(published) => break 'found (request, published),
                Err(error) => { failure.get_or_insert(error); }
            }
        }
        return Err(format!("no font for face {:?}: {}", face.family, failure.map_or_else(String::new, |e| e.to_string())));
    };
    if let Some(error) = failure { warn_once(format!("face family {:?} shows in {:?}: {error}", face.family, request.family)); }
    let mut unresolved: Vec<char> = published.unresolved.iter().map(|&(ch, _)| ch).collect();
    for icon in text.chars().filter(|&c| is_icon(c)) {
        match resolver.resolve_sole(&request, icon) {
            Ok(resolution) => {
                if !frame.fonts.contains_key(&resolution.font.id) { frame.fonts.insert(resolution.font.id, resolution.font.clone()); }
                frame.char_fonts.entry(id).or_default().insert(icon, resolution.glyph.clone());
            }
            Err(error) => {
                warn_once(format!("icon U+{:04X} shows glyphless: {error}", icon as u32));
                unresolved.push(icon);
            }
        }
    }
    for (ch, error) in &published.unresolved { warn_once(format!("U+{:04X} shows glyphless: {error}", *ch as u32)); }
    let font = &frame.fonts[&published.primary];
    let placement = underline_placement(face, font, rules);
    let (ascent_px, descent_px) = (font.ascent_px, font.descent_px);
    if let Some(published) = frame.faces.get_mut(&id) {
        (published.underline_placement, published.underline_thickness) = placement;
        (published.font_ascent, published.font_descent) = (ascent_px.round() as i32, descent_px.round() as i32);
    }
    let font = &frame.fonts[&published.primary];
    let height = (font.ascent_px + font.descent_px).ceil();
    let space = font.space_advance_px;
    let per_glyph = font.glyph_advance.cell_advance_px().is_none();
    let mut cell = font.glyph_advance.cell_advance_px().unwrap_or(space);
    let ascent = font.ascent_px;
    if !cell.is_finite() || cell <= 0.0 {
        warn_once(format!("font {:?} has no positive nominal advance", font.family));
        cell = if space.is_finite() && space > 0.0 { space } else { (pixel_size / 2.0).round().max(1.0) };
    }
    if per_glyph {
        // Dual-width fonts such as Iosevka report per-glyph advances. The
        // nominal space is only the frame column metric; text uses the
        // resolver's exact cached advance, including ASCII.
        // Each printable ASCII character once, in code order, without
        // sorting the face's text.
        let mut seen = [false; 128];
        for byte in text.bytes().filter(|b| b.is_ascii() && !b.is_ascii_control()) { seen[usize::from(byte)] = true; }
        for ch in (0u8..128).filter(|&b| seen[usize::from(b)]).map(char::from) {
            match resolver.resolve_char(&request, ch) {
                Ok(resolved) => {
                    frame.fonts.insert(resolved.font.id, resolved.font);
                    frame.char_fonts.entry(id).or_default().insert(ch, resolved.glyph);
                }
                Err(error) => warn_once(format!("{ch:?} in {:?}: {error}", request.family)),
            }
        }
    }
    // A box's edges outside the text grow the face's row by their width
    // above and below (produce_glyphs: `it->ascent += thick').
    let thick = face.decor.boxed.map_or(0.0, |b| f32::from(b.horizontal.max(0)));
    if let Some(boxed) = face.decor.boxed && let Some(published) = frame.faces.get_mut(&id) {
        use romacs_display_protocol::face::{BoxLineWidth, BoxType};
        published.box_type = BoxType::from_gnu_code(boxed.style).unwrap_or(BoxType::Line);
        published.box_line_width = BoxLineWidth::from_gnu(i32::from(boxed.horizontal));
        published.box_vertical_line_width = Some(BoxLineWidth::from_gnu(i32::from(boxed.vertical.unsigned_abs())));
        published.box_color = boxed.color.map(color);
    }
    Ok(Realized { metrics: (ascent + thick, height + 2.0 * thick, cell, space), primary: published.primary, unresolved })
}

/// The font FACE asks for: its family, size, weight, slant and width.
fn font_request(face: &Face) -> FontRequest {
    let slant = if face.italic { romacs_display_protocol::font::FontSlantKind::Italic } else { romacs_display_protocol::font::FontSlantKind::Normal };
    FontRequest { weight: face.weight, slant, width: face.width, ..FontRequest::new(&face.family, face.pixel_size) }
}

/// The advance painting CH in face ID takes: its resolved glyph's, else
/// the face's nominal CELL.
fn advance(frame: &FrameGlyphBuffer, id: FaceId, ch: char, cell: f32) -> f32 {
    // An icon is one column in any family and size (VM2-MODE-LINE).
    if is_icon(ch) { return cell; }
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
    resolver.set_fontsets(input.fontsets.clone());
    if input.faces.is_empty() { return Err("missing face".into()); }
    validate_runs(&input.text, input.faces.len())?;
    let mut frame = new_frame(1, 1, 1)?;
    let face_text = face_text(input.faces.len(), &[(&input.text, 0)]);
    // A face no character uses answers no row height (0).
    let (metrics, _) = realize_faces(&input.faces, &face_text, |_| false, &mut frame, resolver, 0, Default::default(), &input.no_font)?;
    let pixels = |value: f32| if value.fract() == 0.0 && value >= 0.0 && value <= u32::MAX as f32 { Ok(value as u32) }
        else { Err(format!("fractional or negative advance {value}")) };
    let face_heights = metrics.iter().map(|m| pixels(m.1)).collect::<Result<Vec<_>, _>>()?;
    let face_ascents: Vec<u32> = metrics.iter().map(|m| m.0.round().max(0.0) as u32).collect();
    let mut glyphs = Vec::with_capacity(input.text.utf8.len());
    let mut ascents = Vec::with_capacity(input.text.utf8.len());
    let mut inside = 0;
    // A box run's outside side edges add to its first and last glyphs'
    // advances, as append_window paints them.
    let (boxes, mut box_open) = (shows_box(&input.faces, &input.text), false);
    for (byte, ch) in input.text.utf8.char_indices() {
        if ch == '\n' { glyphs.push((0, face_heights[0])); ascents.push(face_ascents[0]); box_open = false; continue; }
        let index = face_at(&input.text, byte, 0);
        let (_, _, cell, space) = metrics[index];
        let id = FaceId::new(index as u32);
        let shown = shown_at(&input.text, byte, ch, None, &mut inside, &frame, id, cell);
        let step = if boxes { box_step(&input.faces, &input.text, (0, index), byte, ch, shows_glyphs(&shown), box_open, false) } else { BoxStep::NONE };
        box_open = step.open;
        let width = step.lead + step.trail + match shown {
            Shown::Tab | Shown::Stretch(_) | Shown::Image(_) => space,
            Shown::Glyphs(shown) => shown.chars().map(|glyph| advance(&frame, id, glyph, cell)).sum::<f32>(),
            Shown::Composed { advance, .. } => advance,
            Shown::Inside => 0.0,
        };
        glyphs.push((pixels(width)?, face_heights[index]));
        ascents.push(face_ascents[index]);
    }
    Ok(Measured { glyphs, face_heights, ascents, face_ascents })
}
