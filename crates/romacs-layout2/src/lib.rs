//! G1a logical-line layout over owned Seam L data. No evaluator can enter this pass.
#[cfg(test)]
#[path = "selection_order_tests.rs"]
mod selection_order_tests;
#[cfg(test)]
mod alternative_state_tests;
mod fringe;
use fringe::RowMarks;
use romacs_display_protocol::frame_glyphs::GlyphRowRole;
use romacs_display_protocol::{
    Color, DisplayFrameId, DisplayWindowId, FaceId, FrameGlyphBuffer, ParentFrameRect,
    PresentationId, PresentedFramePlacement, Rect,
};
use romacs_font::{FontRequest, FontResolver, FontsetPolicy, publish_face_text};
use romacs_layout_input::{ChildFrameSnapshot, StatusBarSnapshot, TabBarSnapshot, TextSlice, WindowSnapshot, FrameSnapshot, Face, FaceHandle, MeasureInput, Measured, LineWrap, PointPlace, Row, WindowEnd, StretchWidth, stretch_advance, tab_advance};

/// Resolve the actual default monospace geometry before window sizing.
pub fn frame_metrics(face: &Face) -> Result<(u32, u32), String> {
    let frame = layout(&metrics_snapshot(face), 1)?;
    Ok((frame.char_height.ceil() as u32, frame.char_width.ceil() as u32))
}

/// An empty window in FACE alone: what the frame's character cell is.
fn metrics_snapshot(face: &Face) -> WindowSnapshot {
    WindowSnapshot {
        id: 1, width: 1, height: 1, start_byte: 0, point_byte: 0, buffer_version: 0,
        text: TextSlice { utf8: String::new(), face_runs: vec![], raw: vec![] }, mode_line: None,
        mode_line_face: romacs_layout_input::FaceHandle(0), header_line: None, header_line_face: romacs_layout_input::FaceHandle(0),
        vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap: Default::default(), cursor_off: false, stretches: vec![], line_numbers: None, chrome: Default::default(), faces: vec![face.clone()],
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
        append_window(snapshot, &mut frame, &mut self.resolver, 0.0, 0.0, None)?;
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
        ends.push(append_window(&placed.snapshot, &mut frame, resolver, placed.x as f32, placed.y as f32, cursor.filter(owns).map(|(id, color)| (id == placed.snapshot.id, color)))?);
    }
    if let Some(tab_bar) = &snapshot.tab_bar { paint_tab_bar(&mut frame, tab_bar, snapshot.width as f32, resolver)?; }
    if let Some(bar) = &snapshot.status_bar { paint_status_bar(&mut frame, bar, resolver)?; }
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
fn paint_tab_bar(frame: &mut FrameGlyphBuffer, tab_bar: &TabBarSnapshot, width: f32, resolver: &mut FontResolver) -> Result<(), String> {
    let text = &tab_bar.text;
    validate_runs(text, tab_bar.faces.len())?;
    let face = tab_bar.face.0 as usize;
    if face >= tab_bar.faces.len() || text.utf8.chars().any(char::is_control) { return Err("invalid tab bar".into()); }
    let face_base = frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(0);
    let face_text = face_text(tab_bar.faces.len(), &[(text, face)]);
    let metrics = realize_faces(&tab_bar.faces, &face_text, |index| index == face, frame, resolver, face_base)?;
    let bounds = Rect::new(0.0, 0.0, width, tab_bar.height as f32);
    frame.add_background(bounds.x, bounds.y, bounds.width, bounds.height, color(tab_bar.faces[face].background));
    frame.set_draw_context(DisplayWindowId::new(0), GlyphRowRole::TabBar, Some(bounds));
    let mut x = 0.0;
    for (byte, ch) in text.utf8.char_indices() {
        let index = face_at(text, byte, face);
        let id = FaceId::new(face_base + index as u32);
        let (ascent, height, cell, _) = metrics[index];
        let advance = advance(frame, id, ch, cell);
        let background = color(tab_bar.faces[index].background);
        if index != face { frame.add_background(x, 0.0, advance, bounds.height, background); }
        frame.add_char(ch, x, 0.0, advance, height, ascent, false);
        *frame.glyphs.last_mut().and_then(|g| g.face_id_mut()).ok_or("missing tab bar face")? = id;
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

/// The root frame's status bar (VM2-STATUS-BAR): BAR's band in its face's
/// background, then each character in its face on that background, as a
/// window's band paints, in faces published above the windows', clipped
/// to the band. A click finds its character by these glyphs
/// (`status_bar_extents'), which are the bar's only ones and in its
/// text's order.
fn paint_status_bar(frame: &mut FrameGlyphBuffer, bar: &StatusBarSnapshot, resolver: &mut FontResolver) -> Result<(), String> {
    let text = &bar.text;
    validate_runs(text, bar.faces.len())?;
    let face = bar.face.0 as usize;
    if face >= bar.faces.len() || text.utf8.chars().any(char::is_control) { return Err("invalid status bar".into()); }
    let background = bar.faces[face].background;
    let faces: Vec<Face> = bar.faces.iter().map(|f| Face { background, ..f.clone() }).collect();
    let face_base = frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(0);
    let face_text = face_text(faces.len(), &[(text, face)]);
    let metrics = realize_faces(&faces, &face_text, |index| index == face, frame, resolver, face_base)?;
    let bounds = Rect::new(bar.x as f32, bar.y as f32, bar.width as f32, bar.height as f32);
    frame.add_background(bounds.x, bounds.y, bounds.width, bounds.height, color(background));
    frame.set_draw_context(DisplayWindowId::new(0), GlyphRowRole::StatusBar, Some(bounds));
    let mut x = 0.0;
    for (byte, ch) in text.utf8.char_indices() {
        let index = face_at(text, byte, face);
        let id = FaceId::new(face_base + index as u32);
        let (ascent, height, cell, _) = metrics[index];
        let advance = advance(frame, id, ch, cell);
        if x + advance > bounds.width { break; }
        frame.add_char(ch, bounds.x + x, bounds.y, advance, height, ascent, false);
        *frame.glyphs.last_mut().and_then(|g| g.face_id_mut()).ok_or("missing status bar face")? = id;
        x += advance;
    }
    frame.status_bar = Some(bounds);
    Ok(())
}

/// The status bar's characters as FRAME paints them: each one's left edge
/// and width, in its text's order; the characters past the last are
/// clipped.
pub fn status_bar_extents(frame: &FrameGlyphBuffer) -> Vec<(f32, f32)> {
    frame.glyphs.iter().filter_map(|glyph| match glyph {
        romacs_display_protocol::FrameGlyph::Char { row_role: GlyphRowRole::StatusBar, x, width, .. } => Some((*x, *width)),
        _ => None,
    }).collect()
}

/// CHILD of SNAPSHOT as a frame of its own: its windows inside its border,
/// on its background, placed over the root frame at stacking Z. A child
/// window has the cursor only when it is the selected window.
fn layout_child(snapshot: &FrameSnapshot, child: &ChildFrameSnapshot, presentation: u64, z: i32, cursor: Option<(u64, Color)>, resolver: &mut FontResolver) -> Result<(FrameGlyphBuffer, Vec<WindowEnd>), String> {
    if child.id == 1 || child.windows.is_empty() { return Err("invalid child frame".into()); }
    let frame_snapshot = FrameSnapshot {
        font_selection: snapshot.font_selection, font_alternatives: snapshot.font_alternatives.clone(),
        width: child.width, height: child.height, windows: child.windows.clone(), background_alpha: child.background_alpha, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None,
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
    frame.layer = layer_motion(child, &ends);
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
fn layer_motion(child: &ChildFrameSnapshot, ends: &[WindowEnd]) -> romacs_display_protocol::Layer {
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
        selection: child.selection.and_then(|row| selection_band(child, ends.first()?, row)),
        radius: child.radius as f32,
        glow: child.glow.map(color),
        shadow: child.shadow,
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
/// side, the row's height, corners of 4px, in the background of the face
/// at the row's start; none past the rows shown or under a header line.
fn selection_band(child: &ChildFrameSnapshot, end: &WindowEnd, row: u32) -> Option<romacs_display_protocol::SelectionBand> {
    let placed = child.windows.first()?;
    let window = &placed.snapshot;
    let row = end.rows.get(row as usize).filter(|_| window.header_line.is_none())?;
    let face = window.text.face_runs.iter().find(|run| run.bytes.contains(&row.start)).map_or(0, |run| run.face.0 as usize);
    let inset = (placed.x as f32).min(4.0);
    Some(romacs_display_protocol::SelectionBand {
        x: placed.x as f32 - inset, y: placed.y as f32 + row.y, width: window.width as f32 + 2.0 * inset, height: row.height,
        radius: 4.0,
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
    append_window(snapshot, &mut frame, &mut resolver, 0.0, 0.0, None)?;
    Ok(frame)
}

/// Paint SNAPSHOT's window at OX, OY, and report its rows (`WindowEnd`).
fn append_window(snapshot: &WindowSnapshot, frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver, ox: f32, oy: f32, cursor: Option<(bool, Color)>) -> Result<WindowEnd, String> {
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
    let mut stretches = snapshot.stretches.iter().peekable();
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
    // A band's own face sets its height even where no character uses it,
    // as the faces of the fringes' bitmaps set their colours.
    let band_faces: Vec<usize> = bands.iter().filter(|(band, _)| band.is_some()).map(|(_, face)| face.0 as usize)
        .chain(fringe::bitmap_faces(&snapshot.chrome)).collect();
    let metrics = realize_faces(&snapshot.faces, &face_text, |index| band_faces.contains(&index), frame, resolver, face_base)?;
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
    let special = if truncate { '$' } else { '\\' };
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
    let dollar = if snapshot.chrome.left_fringe == 0 { advance(frame, default_id, '$', metrics[0].2) } else { 0.0 };
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
                if visible_end == row_start && fully_visible(y, row_height) { visible_end = byte + 1; }
                rows.push(Row { start: row_start, y, height: row_height });
                marks.push(RowMarks { continued: false, continuation, truncated: hidden, truncated_left: shift > 0.0 });
                if shift > 0.0 && left_mark(shift) > 0.0 && y < body_height && y + metrics[0].1 > 0.0 {
                    glyph(frame, '$', ox + x0, oy + y, metrics[0], default_id)?;
                }
                if let Some(place) = place.as_mut().filter(|_| row_start <= snapshot.point_byte && snapshot.point_byte <= byte) {
                    place.truncated_right = hidden;
                }
                row_start = byte + 1;
            }
            continuation = false;
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
        let shown = match stretch { Some(width) => Shown::Stretch(width), None => shown(ch, snapshot.text.is_raw(byte)) };
        // X on the row and from the line's start (`continued' before it);
        // a TAB stops as from after the line number (x_produce_glyphs).
        let width = |frame: &FrameGlyphBuffer, x: f32, line_x: f32, x0: f32| match &shown {
            Shown::Tab => tab_advance(f64::from(line_x - x0), f64::from(space), snapshot.tab_width) as f32,
            Shown::Stretch(width) => stretch_advance(*width, f64::from(x), f64::from(line_x), (!truncate).then_some(f64::from(last_x))) as f32,
            Shown::Glyphs(glyphs) => glyphs.chars().map(|glyph| advance(frame, id, glyph, cell)).sum(),
        };
        let mut advance_px = width(frame, x, x + continued, x0);
        if measuring { x += advance_px; continue; }
        if !truncate && hpos > 0 && x + advance_px > last_x {
            if glyphs_mark && drawing && y < body_height && y + metrics[0].1 > 0.0 { glyph(frame, special, ox + x, oy + y, metrics[0], default_id)?; }
            if drawing {
                if visible_end == row_start && fully_visible(y, row_height) { visible_end = byte; }
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
            advance_px = width(frame, x0, x0 + continued, x0);
            if at_point { point_rect = Some((x, y, metrics[0].2, row_height, metrics[0].0)); }
        }
        if starting {
            // A start inside a row (the motion refuses one): the row
            // breaks there.
            continuation = true;
            continued += x;
            x0 = gutter(byte).0;
            (drawing, x, y, row_height, hpos) = (true, x0, top, metrics[0].1, 0);
            advance_px = width(frame, x0, x0 + continued, x0);
            if at_point { point_rect = Some((x, y, metrics[0].2, row_height, metrics[0].0)); }
        }
        row_height = row_height.max(height);
        let visible = drawing && body_height > 0.0 && y < body_height && y + height > 0.0 && x - shift < snapshot.width as f32;
        if truncate && x + advance_px > last_x + shift {
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
        let cursor = match &shown {
            Shown::Tab => {
                if visible { frame.add_stretch(ox + x, oy + y, advance_px, height, color(snapshot.faces[index].background), id, false); }
                // GNU's cursor on a stretch is no wider than a column
                // (`x-stretch-cursor' nil).
                advance_px.min(metrics[0].2)
            }
            // A stretch that is not positive is no glyph (nglyphs 0).
            Shown::Stretch(_) => {
                if visible && advance_px > 0.0 { frame.add_stretch(ox + x, oy + y, advance_px, height, color(snapshot.faces[index].background), id, false); }
                advance_px.clamp(0.0, metrics[0].2)
            }
            Shown::Glyphs(glyphs) => {
                let (mut at, mut first) = (x, None);
                for c in glyphs.chars() {
                    let width = advance(frame, id, c, cell);
                    if visible && at - shift < snapshot.width as f32 { glyph_of(frame, c, ox + at, oy + y, width, (ascent, height), id)?; }
                    first.get_or_insert(width);
                    at += width;
                }
                first.unwrap_or(cell)
            }
        };
        if at_point {
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
    if end_byte == text.len() && y < body_height {
        let scrolled = shift > 0.0 && row_start < text.len();
        rows.push(Row { start: row_start, y, height: row_height });
        marks.push(RowMarks { continued: false, continuation, truncated: hidden, truncated_left: scrolled });
        if scrolled && left_mark(shift) > 0.0 && y + metrics[0].1 > 0.0 { glyph(frame, '$', ox + x0, oy + y, metrics[0], default_id)?; }
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
        paint_band(frame, snapshot, band, snapshot.header_line_face, bounds, GlyphRowRole::HeaderLine, (face_base, copy), &metrics)?;
    }
    if let Some(band) = &snapshot.mode_line {
        copy += 1;
        let bounds = Rect::new(band_x, oy + body_height, band_width, snapshot.height as f32 - header_height - body_height);
        paint_band(frame, snapshot, band, snapshot.mode_line_face, bounds, GlyphRowRole::ModeLine, (face_base, copy), &metrics)?;
    }
    Ok(WindowEnd { id: snapshot.id, end: end_byte, visible_end, body_height, rows, point: place })
}

/// Paint BAND, a header or mode line, in BOUNDS: its FACE's background,
/// then its characters in copies of the window's faces on that background,
/// published from FACE_BASE plus COPY face tables on.
#[allow(clippy::too_many_arguments)]
fn paint_band(frame: &mut FrameGlyphBuffer, snapshot: &WindowSnapshot, band: &TextSlice, face: FaceHandle, bounds: Rect,
              role: GlyphRowRole, (face_base, copy): (u32, u32), metrics: &[(f32, f32, f32, f32)]) -> Result<(), String> {
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
    let mut x = 0.0;
    for (byte, ch) in band.utf8.char_indices() {
        let index = face_at(band, byte, default);
        let id = FaceId::new(face_base + offset + index as u32);
        let (ascent, height, cell, _) = metrics[index];
        let advance = advance(frame, id, ch, cell);
        if x < bounds.width {
            frame.add_char(ch, bounds.x + x, bounds.y, advance, height, ascent, false);
            *frame.glyphs.last_mut().and_then(|g| g.face_id_mut()).ok_or("missing mode-line face")? = id;
        }
        x += advance;
    }
    Ok(())
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
enum Shown { Tab, Stretch(StretchWidth), Glyphs(Glyphs) }

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
        for (byte, ch) in slice.utf8.char_indices() {
            let face = &mut text[face_at(slice, byte, *default)];
            match shown(ch, slice.is_raw(byte)) {
                Shown::Tab | Shown::Stretch(_) => face.push(' '),
                Shown::Glyphs(glyphs) => face.extend(glyphs.chars()),
            }
        }
    }
    text
}

/// Publish the FACES something paints into FRAME from FACE_BASE on and
/// resolve the fonts their characters need: each face's ascent, row
/// height, nominal advance and space width. A face no character uses is
/// not published, except the default and those KEEP accepts; its metrics
/// are unused.
fn realize_faces(faces: &[Face], face_text: &[String], keep: impl Fn(usize) -> bool, frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver, face_base: u32) -> Result<Vec<(f32, f32, f32, f32)>, String> {
    let policy = FontsetPolicy::new();
    let mut metrics = Vec::new();
    for (index, face) in faces.iter().enumerate() {
        if face.family.is_empty() || !face.pixel_size.is_finite() || face.pixel_size <= 0.0 {
            return Err("invalid font request".into());
        }
        if index != 0 && !keep(index) && face_text[index].is_empty() {
            metrics.push((0.0, 0.0, 0.0, 0.0));
            continue;
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
    // A face no character uses answers no row height (0).
    let metrics = realize_faces(&input.faces, &face_text, |_| false, &mut frame, resolver, 0)?;
    let pixels = |value: f32| if value.fract() == 0.0 && value >= 0.0 && value <= u32::MAX as f32 { Ok(value as u32) }
        else { Err(format!("fractional or negative advance {value}")) };
    let face_heights = metrics.iter().map(|m| pixels(m.1)).collect::<Result<Vec<_>, _>>()?;
    let mut glyphs = Vec::with_capacity(input.text.utf8.len());
    for (byte, ch) in input.text.utf8.char_indices() {
        if ch == '\n' { glyphs.push((0, face_heights[0])); continue; }
        let index = face_at(&input.text, byte, 0);
        let (_, _, cell, space) = metrics[index];
        let width = match shown(ch, input.text.is_raw(byte)) {
            Shown::Tab | Shown::Stretch(_) => space,
            Shown::Glyphs(shown) => shown.chars().map(|glyph| advance(&frame, FaceId::new(index as u32), glyph, cell)).sum(),
        };
        glyphs.push((pixels(width)?, face_heights[index]));
    }
    Ok(Measured { glyphs, face_heights })
}
