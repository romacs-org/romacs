//! Layout of owned display input (`romacs-layout-input`) into glyphs. No
//! evaluator can enter this pass.
//!
//! A window is laid out by display line: its published text is cut at the
//! newlines that end rows (`line`), each line wrapped into rows from its own
//! state (`wrap`), and the rows kept in a cache by the line's content
//! (`rows`), found again on the next layout and resumed from after an edit.
//! A window's view of the rows, its paint and its row questions are
//! `window`; faces are realised once per content (`faces`). `whole` is the
//! layout of a window from its start's line in one loop, which tests and
//! `ROMACS_LAYOUT_CHECK` compare every window against. The engine also keeps
//! the last frame, for a blink and for a frame whose windows only moved
//! their point.
#[cfg(test)]
#[path = "selection_order_tests.rs"]
mod selection_order_tests;
#[cfg(test)]
mod alternative_state_tests;
mod faces;
mod fringe;
mod px;
mod glyphless;
mod line;
mod rows;
mod whole;
mod window;
mod wrap;
#[cfg(test)]
mod differential_tests;
pub use rows::Counts;
use glyphless::Glyphless;
use fringe::RowMarks;
use romacs_display_protocol::frame_glyphs::GlyphRowRole;
use romacs_display_protocol::{
    Color, DisplayFrameId, DisplayWindowId, FaceId, FrameGlyphBuffer, ParentFrameRect,
    PresentationId, PresentedFramePlacement, Rect,
};
use romacs_font::{FontRequest, FontResolver, publish_face_text};
use romacs_layout_input::{NoFont, ChildFrameSnapshot, PlacedWindowSnapshot, STATUS_BAR_ACCENT, STATUS_BAR_PAD, StatusBarSnapshot, TabBarSnapshot, TextSlice, WindowSnapshot, FrameSnapshot, Face, FaceHandle, MeasureInput, Measured, LineWrap, PointPlace, Row, WindowEnd, StretchWidth, stretch_advance, tab_advance};

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
        vscroll_px: 0, hscroll: Default::default(), tab_width: 8, wrap: Default::default(), cursor_off: false, cursor: Default::default(), stretches: vec![], images: vec![], wrap_flips: vec![], line_numbers: None, chrome: Default::default(), faces: vec![face.clone()],
    }
}

/// Stateful font discovery cache, the faces realised and the lines laid
/// out so far, and the last frame laid out. Contains only owned font and
/// glyph data, never VM values.
pub struct LayoutEngine { store: Store, last: Option<Laid> }

/// What layouts keep: the fonts, the faces realised under the font rules
/// the resolver last retired its answers for (RETIRED), and the lines.
/// CHECK lays every window out whole beside and counts where the two
/// differ (`ROMACS_LAYOUT_CHECK`).
struct Store {
    resolver: FontResolver,
    faces: faces::Faces,
    cache: rows::Cache,
    retired: u64,
    check: bool,
    mismatches: u64,
}

impl Store {
    fn new() -> Result<Self, String> {
        let resolver = FontResolver::new().map_err(|e| e.to_string())?;
        let check = cfg!(test) || std::env::var_os("ROMACS_LAYOUT_CHECK").is_some_and(|v| v != "0");
        Ok(Store { retired: resolver.retired(), resolver, faces: faces::Faces::default(), cache: rows::Cache::default(), check, mismatches: 0 })
    }

    /// Adopt the font rules SNAPSHOT lays out under: what was realised or
    /// laid out under others goes.
    fn rules(&mut self, selection: romacs_layout_input::FontSelectionState, alternatives: &romacs_layout_input::FontAlternativeState,
             fontsets: &std::sync::Arc<romacs_layout_input::FontsetState>, no_font: &NoFont) {
        self.resolver.set_selection_state(selection);
        self.resolver.set_alternative_state(alternatives.clone());
        self.resolver.set_fontsets(fontsets.clone());
        let renamed = self.faces.set_no_font(no_font);
        if renamed || self.resolver.retired() != self.retired {
            self.retired = self.resolver.retired();
            self.faces.clear();
            self.cache.clear();
        }
    }

    fn kept(&mut self) -> window::Kept<'_> {
        window::Kept { cache: &mut self.cache, faces: &mut self.faces, resolver: &mut self.resolver }
    }

    /// SNAPSHOT's window laid out, and painted into FRAME at OX, OY with
    /// CURSOR when given; under CHECK, laid out whole beside and compared.
    fn window(&mut self, snapshot: &WindowSnapshot, frame: Option<&mut FrameGlyphBuffer>, (ox, oy): (f32, f32), cursor: Option<(bool, Color)>, no_font: &NoFont) -> Result<WindowEnd, String> {
        let Some(frame) = frame else {
            let end = window::append_window(snapshot, None, &mut self.kept(), ox, oy, None, window::Paint::No)?;
            if self.check {
                let mut whole = new_frame(snapshot.width.max(1), snapshot.height.max(1), 1)?;
                let oracle = whole::append_window(snapshot, &mut whole, &mut self.resolver, ox, oy, None, no_font)?;
                if oracle != end { self.mismatch(&format!("window {} ends {end:?}, whole {oracle:?}", snapshot.id)); }
            }
            return Ok(end);
        };
        let before = self.check.then(|| frame.clone());
        let end = window::append_window(snapshot, Some(frame), &mut self.kept(), ox, oy, cursor, window::Paint::Whole)?;
        if let Some(mut whole) = before {
            let oracle = whole::append_window(snapshot, &mut whole, &mut self.resolver, ox, oy, cursor, no_font)?;
            if oracle != end { self.mismatch(&format!("window {} ends {end:?}, whole {oracle:?}", snapshot.id)); }
            else if whole != *frame { self.mismatch(&format!("window {} paints otherwise than whole: {}", snapshot.id, differ(&whole, frame))); }
        }
        Ok(end)
    }

    fn mismatch(&mut self, what: &str) {
        self.mismatches += 1;
        warn_once(what.chars().take(2000).collect());
        assert!(!cfg!(test) && std::env::var_os("ROMACS_LAYOUT_CHECK").is_none_or(|v| v != "panic"), "{what}");
    }
}

/// Where frames A and B, which should be equal, first differ.
fn differ(a: &FrameGlyphBuffer, b: &FrameGlyphBuffer) -> String {
    if a.glyphs.len() != b.glyphs.len() || a.glyphs != b.glyphs {
        let at = a.glyphs.iter().zip(&b.glyphs).position(|(a, b)| a != b).unwrap_or(a.glyphs.len().min(b.glyphs.len()));
        return format!("glyph {at} of {}/{}: {:?} / {:?}", a.glyphs.len(), b.glyphs.len(), a.glyphs.get(at), b.glyphs.get(at));
    }
    if a.faces != b.faces { return format!("faces {:?} / {:?}", a.faces.keys().collect::<Vec<_>>(), b.faces.keys().collect::<Vec<_>>()); }
    if a.char_fonts != b.char_fonts { return format!("char fonts {:?} / {:?}", a.char_fonts, b.char_fonts); }
    if a.fonts.len() != b.fonts.len() { return format!("fonts {} / {}", a.fonts.len(), b.fonts.len()); }
    if a.window_cursors != b.window_cursors { return format!("cursors {:?} / {:?}", a.window_cursors, b.window_cursors); }
    format!("elsewhere: {:?}", (a.char_width, a.char_height, b.char_width, b.char_height))
}

/// A frame laid out with every cursor, as if none were turned off.
struct Laid {
    snapshot: FrameSnapshot,
    cursor: Option<(u64, Color)>,
    frame: FrameGlyphBuffer,
    /// Where in FRAME its windows end and its bars begin.
    bars: Bars,
    ends: Vec<WindowEnd>,
}

/// What a frame held when its windows were laid out and its bars were
/// not yet: its glyphs, its rounded fills, the first face id after its
/// windows' faces, and its fonts.
#[derive(Clone)]
struct Bars {
    glyphs: usize,
    fills: usize,
    faces: u32,
    fonts: Vec<romacs_display_protocol::font::ResolvedFontId>,
}

impl Bars {
    fn of(frame: &FrameGlyphBuffer) -> Self {
        Bars { glyphs: frame.glyphs.len(), fills: frame.rounded_fills.len(),
            faces: frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(0), fonts: frame.fonts.keys().copied().collect() }
    }

    /// FRAME as it was before its bars were painted.
    fn undo(&self, frame: &mut FrameGlyphBuffer) {
        frame.glyphs.truncate(self.glyphs);
        frame.rounded_fills.truncate(self.fills);
        let first = self.faces;
        frame.faces.retain(|id, _| id.get() < first);
        frame.char_fonts.retain(|id, _| id.get() < first);
        frame.shaped_clusters.retain(|id, _| id.get() < first);
        frame.fonts.retain(|id, _| self.fonts.contains(id));
        (frame.status_bar, frame.status_bar_places) = (None, vec![]);
    }
}
impl LayoutEngine {
    pub fn new() -> Result<Self, String> {
        Ok(Self { store: Store::new()?, last: None })
    }
    /// SNAPSHOT laid out at PRESENTATION. A frame that differs from the last
    /// one only in which cursors are off (GNU's blink, `internal-show-cursor`)
    /// is not laid out again: the last layout shows it, less those cursors.
    pub fn layout_frame(&mut self, snapshot: &FrameSnapshot, presentation: u64, cursor: Option<(u64, Color)>) -> Result<(FrameGlyphBuffer, Vec<WindowEnd>), String> {
        let reusable = self.last.as_ref().is_some_and(|last| last.cursor == cursor && last.snapshot.lays_out_like(snapshot));
        // A frame whose windows differ only in where point is, where point
        // moves no glyph, is the last one's windows with their cursors
        // placed anew and the bars painted again.
        let moved = !reusable && self.last.as_ref().is_some_and(|last| last.cursor == cursor && last.snapshot.windows_lay_out_like(snapshot, false)
            && (last.snapshot.tab_bar.is_some(), last.snapshot.status_bar.is_some()) == (snapshot.tab_bar.is_some(), snapshot.status_bar.is_some()));
        if moved {
            let last = self.last.as_mut().expect("compared above");
            let bars = last.snapshot.tab_bar != snapshot.tab_bar || last.snapshot.status_bar != snapshot.status_bar;
            last.ends = cursors_moved(snapshot, &mut last.frame, &last.bars, cursor, &mut self.store)?;
            // Only cursors moved: the same layout. Bars painted again: another.
            if bars { last.frame.layout_identity = next_layout_identity(); }
            if self.store.check {
                let (mut whole, _) = layout_frame_inner(snapshot, last.frame.presentation_id.get(), cursor, &mut self.store)?;
                whole.0.layout_identity = last.frame.layout_identity;
                if whole.0 != last.frame { self.store.mismatch(&format!("a frame of moved cursors paints otherwise than a whole one: {}", differ(&whole.0, &last.frame))); }
            }
            // It is SNAPSHOT now: its points and bars.
            for (kept, placed) in last.snapshot.windows.iter_mut().zip(&snapshot.windows) { kept.snapshot.point_byte = placed.snapshot.point_byte; }
            if bars { (last.snapshot.tab_bar, last.snapshot.status_bar) = (snapshot.tab_bar.clone(), snapshot.status_bar.clone()); }
        } else if !reusable {
            let ((mut frame, bars), ends) = layout_frame_inner(snapshot, presentation, cursor, &mut self.store)?;
            frame.layout_identity = next_layout_identity();
            self.last = Some(Laid { snapshot: snapshot.clone(), cursor, frame, bars, ends });
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
            let (frame, child_ends) = layout_child(snapshot, child, presentation, z as i32, cursor, &mut self.store)?;
            frames.push(frame);
            ends.extend(child_ends);
        }
        Ok((frames, ends))
    }
    /// What laying SNAPSHOT out reports, presenting nothing and leaving the
    /// last painted frame, which a cursor-only repaint reuses, in place.
    pub fn lay_out(&mut self, snapshot: &FrameSnapshot) -> Result<Vec<WindowEnd>, String> {
        let store = &mut self.store;
        store.rules(snapshot.font_selection, &snapshot.font_alternatives, &snapshot.fontsets, &snapshot.no_font);
        snapshot.windows.first().ok_or("empty frame")?;
        store.cache.begin();
        let ends = placed_windows(snapshot).map(|placed| placed.and_then(|placed| store.window(&placed.snapshot, None, (px::of(placed.x), px::of(placed.y)), None, &snapshot.no_font))).collect();
        store.cache.finish();
        ends
    }
    /// What the layouts so far did (`Counts`), and how often a layout
    /// checked against the whole one differed from it.
    pub fn counts(&self) -> (Counts, u64) { (self.store.cache.counts, self.store.mismatches) }
    /// The host layout hook: INPUT's geometry as painting would lay it out.
    pub fn measure(&mut self, input: &MeasureInput) -> Result<Measured, String> {
        whole::measure_inner(input, &mut self.store.resolver)
    }
    /// `frame_metrics`, from this engine's fonts.
    pub fn frame_metrics(&mut self, face: &Face) -> Result<(u32, u32), String> {
        let frame = self.layout(&metrics_snapshot(face), 1)?;
        Ok((frame.char_height.ceil() as u32, frame.char_width.ceil() as u32))
    }
    /// The files this engine's answers came from (`FontResolver::font_inputs`).
    pub fn font_inputs(&self) -> Vec<String> {
        self.store.resolver.font_inputs()
    }
    pub fn layout(&mut self, snapshot: &WindowSnapshot, presentation: u64) -> Result<FrameGlyphBuffer, String> {
        let mut frame = new_frame(snapshot.width, snapshot.height, presentation)?;
        frame.background = color(snapshot.faces.first().ok_or("missing face")?.background);
        self.store.cache.begin();
        self.store.window(snapshot, Some(&mut frame), (0.0, 0.0), None, &NoFont::default())?;
        self.store.cache.finish();
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

/// SNAPSHOT laid out in FRAME, laid out from a snapshot whose windows
/// differ only in where point is: its windows' glyphs as they are (FRAME
/// before BARS), their cursors placed anew, its bars painted again.
fn cursors_moved(snapshot: &FrameSnapshot, frame: &mut FrameGlyphBuffer, bars: &Bars, cursor: Option<(u64, Color)>, store: &mut Store) -> Result<Vec<WindowEnd>, String> {
    store.rules(snapshot.font_selection, &snapshot.font_alternatives, &snapshot.fontsets, &snapshot.no_font);
    bars.undo(frame);
    frame.window_cursors.clear();
    let mut ends = Vec::with_capacity(snapshot.windows.len());
    store.cache.begin();
    for placed in &snapshot.windows {
        let owns = |&(id, _): &(u64, Color)| placed.snapshot.mode_line.is_some() || id == placed.snapshot.id;
        let cursor = cursor.filter(owns).map(|(id, color)| (id == placed.snapshot.id, color));
        ends.push(window::append_window(&placed.snapshot, Some(frame), &mut store.kept(), px::of(placed.x), px::of(placed.y), cursor, window::Paint::Cursor)?);
    }
    store.cache.finish();
    paint_bars(frame, snapshot, &mut store.resolver)?;
    Ok(ends)
}

/// No cursor in a window of SNAPSHOT whose cursor `internal-show-cursor`
/// turned off, which is a blink, or whose buffer has none, which is not.
fn hide_cursors_off(frame: &mut FrameGlyphBuffer, snapshot: &FrameSnapshot) {
    let ids = |hidden: fn(&WindowSnapshot) -> bool| -> Vec<i64> {
        snapshot.windows.iter().filter(|w| hidden(&w.snapshot)).map(|w| w.snapshot.id as i64).collect()
    };
    let blinked = ids(|w| w.cursor_off);
    frame.cursor_blinked_off = frame.window_cursors.iter().any(|c| c.active && blinked.contains(&c.window_id.get()));
    frame.window_cursors.retain(|c| !blinked.contains(&c.window_id.get()));
}

/// SNAPSHOT laid out with every cursor its windows own, on or off.
fn layout_frame_inner(snapshot: &FrameSnapshot, presentation: u64, cursor: Option<(u64, Color)>, store: &mut Store) -> Result<((FrameGlyphBuffer, Bars), Vec<WindowEnd>), String> {
    store.rules(snapshot.font_selection, &snapshot.font_alternatives, &snapshot.fontsets, &snapshot.no_font);
    let first = snapshot.windows.first().ok_or("empty frame")?;
    let mut frame = new_frame(snapshot.width, snapshot.height, presentation)?;
    frame.background = color(first.snapshot.faces.first().ok_or("missing face")?.background);
    let mut ends = Vec::with_capacity(snapshot.windows.len());
    store.cache.begin();
    for placed in placed_windows(snapshot) {
        let placed = placed?;
        // Windows with a mode line always own a cursor; the minibuffer (no mode
        // line) only while it is the selected window, i.e. a prompt is active.
        let owns = |&(id, _): &(u64, Color)| placed.snapshot.mode_line.is_some() || id == placed.snapshot.id;
        let cursor = cursor.filter(owns).map(|(id, color)| (id == placed.snapshot.id, color));
        ends.push(store.window(&placed.snapshot, Some(&mut frame), (px::of(placed.x), px::of(placed.y)), cursor, &snapshot.no_font)?);
    }
    store.cache.finish();
    let bars = Bars::of(&frame);
    paint_bars(&mut frame, snapshot, &mut store.resolver)?;
    Ok(((frame, bars), ends))
}

/// SNAPSHOT's tab bar, status bar and panes over its windows in FRAME.
fn paint_bars(frame: &mut FrameGlyphBuffer, snapshot: &FrameSnapshot, resolver: &mut FontResolver) -> Result<(), String> {
    if let Some(tab_bar) = &snapshot.tab_bar { paint_tab_bar(frame, tab_bar, snapshot.width as f32, resolver, &snapshot.no_font)?; }
    if let Some(bar) = &snapshot.status_bar { paint_status_bar(frame, bar, snapshot.chrome.focus, resolver, &snapshot.no_font)?; }
    frame.panes = panes(snapshot);
    Ok(())
}

/// SNAPSHOT's windows, each refused when its id repeats or its extent
/// does not fit the frame.
fn placed_windows(snapshot: &FrameSnapshot) -> impl Iterator<Item = Result<&PlacedWindowSnapshot, String>> {
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
    snapshot.windows.iter().map(move |placed| {
        if !ids.insert(placed.snapshot.id)
            || !fits(placed.x, placed.snapshot.width, snapshot.width)
            || !fits(placed.y, placed.snapshot.height, snapshot.height) {
            return Err("invalid or duplicate placed window".into());
        }
        Ok(placed)
    })
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
    let face_text = whole::face_text(tab_bar.faces.len(), &[(text, face)]);
    let (metrics, glyphless) = whole::realize_faces(&tab_bar.faces, &face_text, |index| index == face, frame, resolver, face_base, Default::default(), no_font)?;
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

/// The WCAG 2 contrast ratio of colours A and B, from 1 to 21.
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let luminance = |rgb: [u8; 3]| {
        let [r, g, b] = rgb.map(|c| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
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

/// A band whose painter reads its marks (`BandMarks`): the status bar, or
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
/// text in the pill's foreground), the part after its `right-align`
/// marker flush with the right edge (`BandMarks::places`). Where each
/// character went, from the band's left edge.
fn paint_marked(frame: &mut FrameGlyphBuffer, band: &Marked, owner: (DisplayWindowId, GlyphRowRole), resolver: &mut FontResolver, no_font: &NoFont) -> Result<Vec<Option<(f32, f32)>>, String> {
    let (text, face, bounds) = (band.text, band.face, band.bounds);
    validate_runs(text, band.faces.len())?;
    if face >= band.faces.len() || text.utf8.chars().any(char::is_control) { return Err("invalid marked band".into()); }
    let background = band.faces[face].background;
    let faces: Vec<Face> = band.faces.iter().map(|f| Face { background, ..f.clone() }).collect();
    let face_base = frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(0);
    let mut face_text = whole::face_text(faces.len(), &[(text, face)]);
    for &(byte, icon) in &band.marks.icons { face_text[face_at(text, byte, face)].push(icon); }
    let (metrics, glyphless) = whole::realize_faces(&faces, &face_text, |index| index == face, frame, resolver, face_base, Default::default(), no_font)?;
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
    let mut pill_text: Vec<(std::ops::Range<usize>, [u8; 3], [u8; 3])> = vec![];
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
        if let Some(text) = pill.foreground { pill_text.push((first..end, text, pill.background)); }
    }
    // A pill's text colour: copies of the faces it uses, in that foreground.
    // Above every face realized for the band, its glyphless ones too.
    let mut next = frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(face_base).max(glyphless.end);
    let mut recoloured: std::collections::HashMap<(usize, [u8; 3]), FaceId> = std::collections::HashMap::new();
    let mut ids: Vec<FaceId> = chars.iter().map(|c| FaceId::new(face_base + c.2 as u32)).collect();
    for (range, foreground, background) in &pill_text {
        for i in range.clone() {
            // An icon in a colour of its own keeps it, where it reads.
            let own = band.faces[chars[i].2].foreground;
            if is_icon(chars[i].1) && own != band.faces[face].foreground && contrast(own, *background) >= 3.0 { continue; }
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
    let mut filled = bounds.x + band.accent.map_or(0.0, |_| STATUS_BAR_ACCENT);
    for (i, (&(_, shown, index, _), place)) in chars.iter().zip(&places).enumerate() {
        let Some((x, width)) = *place else { continue };
        if centre_marker.contains(&i) || right_marker.contains(&i) { continue; }
        let (ascent, height) = if boxes { (row_ascent, bounds.height) } else { (metrics[index].0, metrics[index].1) };
        let left = bounds.x + x;
        fill(frame, &mut run, filled, left, background);
        let own = faces[index].decor.boxed;
        if own.is_none() && band_box.is_some() {
            fill(frame, &mut run, left, left + width, background);
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
fn layout_child(snapshot: &FrameSnapshot, child: &ChildFrameSnapshot, presentation: u64, z: i32, cursor: Option<(u64, Color)>, store: &mut Store) -> Result<(FrameGlyphBuffer, Vec<WindowEnd>), String> {
    if child.id == 1 || child.windows.is_empty() { return Err("invalid child frame".into()); }
    let frame_snapshot = FrameSnapshot {
        font_selection: snapshot.font_selection, font_alternatives: snapshot.font_alternatives.clone(), fontsets: snapshot.fontsets.clone(),
        width: child.width, height: child.height, windows: child.windows.clone(), background_alpha: child.background_alpha, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None,
        no_font: snapshot.no_font.clone(),
    };
    let cursor = cursor.filter(|(id, _)| child.windows.iter().any(|w| w.snapshot.id == *id));
    let ((mut frame, _), ends) = layout_frame_inner(&frame_snapshot, presentation, cursor, store)?;
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
    LayoutEngine::new()?.layout(snapshot, presentation)
}

/// Paint BAND, a header or mode line, in BOUNDS: its FACE's background,
/// then its characters in copies of the window's faces on that background,
/// published from FACE_BASE plus COPY face tables on. A character STRETCHES
/// names is that stretch, blank (produce_stretch_glyph), its `:align-to`
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
        settle_row(frame, first_glyph, bounds.y, bounds.height, true, &[], metrics, face_base + offset, (&mut None, 0.0));
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

/// One face's font metrics as the row loop collects them.
type FaceMetrics = (f32, f32, f32, f32);
/// The point's cursor rectangle as the row loop tracks it.
type CursorRect = (f32, f32, f32, f32, f32);

/// Settle the row at TOP, HEIGHT tall, whose glyphs start at FROM, on one
/// baseline, as produce_glyphs's row ascent is the largest of its glyphs'
/// (a boxed glyph's grown by its box's top): each character and each
/// stretch shorter than the row moves down by that ascent less its own
/// (the ascent METRICS[face - FACE_BASE] gives a stretch); the row's new height.  Only a
/// row with a box or characters of another font's metrics (BOXED) is settled.
#[allow(clippy::too_many_arguments, reason = "one row's settling inputs; the callers pass them straight from the row loop")]
fn settle_row(frame: &mut FrameGlyphBuffer, from: usize, top: f32, height: f32, boxed: bool, images: &[(usize, f32)],
              metrics: &[FaceMetrics], face_base: u32, (cursor, row_y): (&mut Option<CursorRect>, f32)) -> f32 {
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
            FrameGlyph::Stretch { y, height: h, .. } | FrameGlyph::Surface { y, height: h, .. } => {
                *y += shift;
                bottom = bottom.max(*y + *h - top);
            }
            FrameGlyph::Image { y, slot_rect, box_rect, .. } => {
                *y += shift;
                slot_rect.y += shift;
                box_rect.y += shift;
                bottom = bottom.max(slot_rect.y + slot_rect.height - top);
            }
            _ => {}
        }
    }
    // The cursor sits on its glyph: down by as much, within this row.
    if let Some(cursor) = cursor.as_mut().filter(|c| c.1 == row_y) {
        let shift = (row_ascent - cursor.4).max(0.0);
        cursor.1 += shift;
        cursor.3 = cursor.3.min(bottom - shift);
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

/// Whether the character CH at BYTE of TEXT, beyond ASCII, shows as an
/// escape (a raw byte, or a control character): ASCII glyphs of its face,
/// which rise as the face does, whatever font the character it stands
/// for is drawn in elsewhere.
fn escaped(text: &TextSlice, byte: usize, ch: char) -> bool {
    !ch.is_ascii() && (ch.is_control() || text.is_raw(byte))
}

/// Whether SHOWN draws glyphs (a composed sequence's included), which a
/// box run's edges belong to, rather than a stretch.
fn shows_glyphs(shown: &Shown) -> bool {
    matches!(shown, Shown::Glyphs(_) | Shown::Composed { .. } | Shown::Inside)
}

/// Publish the one glyph a sequence of a single character and its
/// selectors (VS-16's `❤️`) shows in face ID, when it has one: the
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

/// The face table index painting SLICE's character at BYTE.
fn face_at(slice: &TextSlice, byte: usize, default: usize) -> usize {
    // Runs are ordered and disjoint (`validate_runs`).
    let at = slice.face_runs.partition_point(|run| run.bytes.end <= byte);
    slice.face_runs.get(at).filter(|run| run.bytes.contains(&byte)).map_or(default, |run| run.face.0 as usize)
}

/// Where FACE's underline goes in FONT, and how thick it is, as xterm.c
/// x_draw_glyph_string decides: the font's thickness, else 1; at the
/// descent line (less `:position` pixels) under RULES' at-descent-line or a
/// `:position`, else the font's own position under `use_position` when it
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

/// A face's ascent, height, nominal advance and space width.
type Metrics = (f32, f32, f32, f32);

/// Note WHAT once per process: a display-time font failure is degraded,
/// never fatal, and is reported the first time it happens.
fn warn_once(what: String) {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);
    let Ok(mut seen) = SEEN.lock() else { return };
    let seen = seen.get_or_insert_with(Default::default);
    if seen.len() < 256 && seen.insert(what.clone()) { eprintln!("romacs-layout: {what}"); }
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
