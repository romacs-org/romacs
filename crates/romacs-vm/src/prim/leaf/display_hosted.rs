//! The display elements of a window a host lays out (VM2-WTPS1). A
//! window's snapshot publishes the elements GNU's iterator delivers
//! (`Leaf::shown_elements`): its buffer characters, `display' strings and
//! spaces, overlay strings and ellipses, with invisible text left out, each
//! in the face GNU shows it in. The host paints them one glyph each, a TAB
//! and a space as stretches, and continues or truncates a row where GNU's
//! iterator does, though never at a word boundary (`word-wrap'). The same
//! elements measure here: each becomes a `Shape::Pixel` glyph with the
//! advance and height the host's own layout gives it, or a
//! `Shape::PixelTab` or `Shape::PixelStretch` from its face, so the motion
//! measures in the host's pixels. What the host does not draw as GNU does
//! (escapes and display-table glyphs, glyphless characters, line and wrap
//! prefixes) is refused: the host would not show what GNU shows there.
use super::*;
use super::display_elements::{Source, View};
use super::display_motion::{Elem, Elements, Lines, Method, Shape, Slot, R};
use romacs_layout_input::StretchWidth;

/// The elements one host measurement covers past the first it needs.
const CHUNK: usize = 4096;

fn unsupported(what: &str) -> Fault { Fault::Error(format!("VM2 window-text-pixel-size {what} in a displayed window is unsupported")) }
fn not_as_gnu() -> Fault { unsupported("of text the host does not lay out as GNU displays it") }

/// The face a display element shows in: the buffer's at a position (GNU
/// face_at_buffer_position), or a string's own at STRPOS over that
/// (face_at_string_position, the buffer text's face as its base), over the
/// text's alone for an OVERLAY's string (face_for_overlay_string).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FaceKey { Buffer(i64), String { string: Value, strpos: usize, charpos: i64, overlay: bool },
    /// A display-table glyph's face over the buffer's (merge_faces).
    Glyph { face: Value, charpos: i64 } }

/// One element as a host shows it: its character, the face it shows in,
/// the buffer position it is at, what it is there and whether it REPLACES
/// the buffer's text there (the text itself, or what its `display' shows;
/// not an overlay string's), so point can show on it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shown { pub c: char, pub face: FaceKey, pub charpos: i64, pub kind: ShownKind, pub replaces: bool }

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ShownKind {
    /// The buffer's own character at CHARPOS.
    Buffer,
    /// A string's character or an ellipsis glyph at CHARPOS; CURSOR when
    /// its `cursor' property asks for point there.
    Other { cursor: bool },
    /// A `(space ...)' a placeholder space stands for.
    Stretch(StretchWidth),
    /// A `:relative-width' FACTOR space, as wide as the host draws C.
    Relative { factor: f64, c: char },
}

/// How the host shows ELEM: None where it shows nothing of it (outside
/// the text area), else the character it paints, or `Err' where it does
/// not show what GNU does.
fn host_char(source: &mut Source<'_, '_>, e: &Elem) -> R<Option<Option<char>>> {
    if !e.text_area { return Ok(None); }
    Ok(Some(match (e.shape, e.method) {
        (Shape::Glyph { c, .. }, Method::Buffer | Method::String) => char::from_u32(c),
        (Shape::Tab, Method::Buffer | Method::String) => Some('\t'),
        (Shape::Stretch { .. } | Shape::Align { .. }, _) => Some(' '),
        // Measured as the glyph it is relative to.
        (Shape::RelativeStretch { c, .. }, _) => char::from_u32(c),
        (Shape::Glyph { c, .. }, Method::Vector { .. }) if e.ellipsis.is_some() => char::from_u32(c),
        // An escape or a display table's glyphs stand for the character
        // there, which the host draws itself.
        (_, Method::Vector { last, .. }) => {
            if !last { return Ok(Some(None)); }
            let c = match e.strpos {
                Some(k) if e.object != Value::NIL => source.leaf().vm.str_view(e.object).and_then(|v| v.chars().nth(k)),
                _ => source.char_after(e.charpos)?,
            };
            return Ok(Some(c.and_then(char::from_u32)));
        }
        _ => None,
    }))
}

fn face_key(e: &Elem) -> FaceKey {
    if let Some(face) = e.ellipsis.filter(|&f| f != Value::NIL) { return FaceKey::Glyph { face, charpos: e.charpos }; }
    match e.strpos {
        Some(strpos) if e.object != Value::NIL => FaceKey::String { string: e.object, strpos, charpos: e.charpos, overlay: !e.from_display },
        _ => FaceKey::Buffer(e.charpos),
    }
}

fn stretch_width(shape: Shape) -> Option<StretchWidth> {
    match shape {
        Shape::Stretch { width, zero_ok } => Some(StretchWidth::Width { width, zero_ok }),
        Shape::Align { target } => Some(StretchWidth::AlignTo(target)),
        _ => None,
    }
}

impl<'a> Leaf<'a> {
    /// The elements VIEW's window shows from FROM, a line start, through
    /// the last before TO (and the strings at TO, when it is ZV), as a
    /// snapshot publishes them for the host to paint.
    pub(super) fn shown_elements(&mut self, view: View, from: i64, to: i64) -> R<Vec<Shown>> {
        let zv = view.zv;
        let cursor = self.intern("cursor");
        let mut source = Source::new(self, view, from)?;
        let mut out = vec![];
        for index in 0.. {
            let Some(e) = source.slot(index)?.elem else { break };
            if e.charpos > to || e.charpos == to && (to < zv || e.method == Method::Buffer) { break; }
            let Some(Some(c)) = host_char(&mut source, &e)? else { continue };
            // A space replaces when the buffer's display made it, not a string's.
            let (kind, replaces) = match stretch_width(e.shape) {
                Some(width) => (ShownKind::Stretch(width), e.object == Value::NIL),
                None if let Shape::RelativeStretch { factor, .. } = e.shape => (ShownKind::Relative { factor, c }, e.object == Value::NIL),
                None if e.object == Value::NIL && e.ellipsis.is_none() && matches!(e.method, Method::Buffer | Method::Vector { .. }) => (ShownKind::Buffer, true),
                None => {
                    let asks = match e.strpos { Some(k) if e.object != Value::NIL && !e.from_display =>
                        source.leaf().vm.property_textget(e.object, k as i64, cursor)? != Value::NIL, _ => false };
                    (ShownKind::Other { cursor: asks }, e.from_display)
                }
            };
            out.push(Shown { c, face: face_key(&e), charpos: e.charpos, kind, replaces });
        }
        Ok(out)
    }
}

pub(super) struct Hosted<'l, 'a> {
    source: Source<'l, 'a>,
    /// The host's advance and own row height for the elements from BASE
    /// on, by index since the last reset.
    base: usize,
    glyphs: Vec<(i64, i64)>,
    /// The default face's row height, which every host row reaches.
    pub default_height: i64,
}

impl<'l, 'a> Hosted<'l, 'a> {
    pub(super) fn new(source: Source<'l, 'a>) -> Self {
        Hosted { source, base: 0, glyphs: vec![], default_height: 0 }
    }
    pub(super) fn source(&mut self) -> &mut Source<'l, 'a> { &mut self.source }
    /// Deliver from START, a line start, again.
    pub(super) fn reset(&mut self, start: i64) -> R<()> {
        (self.base, self.glyphs) = (0, vec![]);
        self.source.reset(start)
    }
    /// The host's advance and own height for element INDEX.
    pub(super) fn glyph(&mut self, index: usize) -> R<(i64, i64)> {
        if index < self.base || index >= self.base + self.glyphs.len() { self.measure(index)?; }
        self.glyphs.get(index - self.base).copied().ok_or_else(|| Fault::Error("host measurement misses an element".into()))
    }
    /// Ask the host for the elements from INDEX through the end of the
    /// line CHUNK elements on, or up to the first the iterator refuses,
    /// which refuses when the motion reaches it.
    fn measure(&mut self, index: usize) -> R<()> {
        let mut shown = vec![];
        for at in index.. {
            let slot = match self.source.slot(at) {
                Ok(slot) => slot,
                Err(refused) if at == index => return Err(refused),
                Err(_) => break,
            };
            let Some(e) = slot.elem else { break };
            let c = match host_char(&mut self.source, &e)? { Some(Some(c)) => c, _ => ' ' };
            shown.push((c, face_key(&e)));
            if at - index >= CHUNK && c == '\n' { break; }
        }
        let window = self.source.window();
        let measured = self.source.leaf().vm.host_measure_elements(window, &shown).map_err(Fault::Error)?;
        self.base = index;
        self.glyphs = measured.glyphs.into_iter().map(|(w, h)| (i64::from(w), i64::from(h))).collect();
        self.default_height = i64::from(measured.face_heights[0]);
        Ok(())
    }
    /// The default face's row height and the advance of C in it.
    pub(super) fn default_advance(&mut self, c: char) -> R<(i64, i64)> {
        let (advance, height) = self.source.leaf().vm.host_default_advance(c).map_err(Fault::Error)?;
        self.default_height = i64::from(height);
        Ok((i64::from(advance), i64::from(height)))
    }
}

impl Elements for Hosted<'_, '_> {
    fn line_at(&mut self, index: usize) -> R<i64> { self.source.line_at(index) }
    fn slot(&mut self, index: usize) -> R<Slot> {
        let slot = self.source.slot(index)?;
        let Some(e) = slot.elem else { return Ok(slot) };
        let Some(c) = host_char(&mut self.source, &e)? else { return Ok(slot) };
        if c.is_none() || matches!(e.method, Method::Vector { .. }) && e.ellipsis.is_none() { return Err(not_as_gnu()); }
        let (width, height) = self.glyph(index)?;
        let height = height.max(self.default_height);
        let shape = match e.shape {
            Shape::Glyph { c, .. } => Shape::Pixel { c, width, height },
            Shape::Tab => Shape::PixelTab { space: width, height },
            // produce_stretch_glyph: NUMVAL (prop) * it2.pixel_width.
            Shape::RelativeStretch { factor, .. } => Shape::PixelStretch {
                width: StretchWidth::Width { width: (factor * width as f64) as i64, zero_ok: false }, unnumbered: None, height },
            shape => Shape::PixelStretch { width: stretch_width(shape).ok_or_else(not_as_gnu)?, unnumbered: e.unnumbered.and_then(stretch_width), height },
        };
        Ok(Slot { pre: slot.pre, elem: Some(Elem { shape, ..e }) })
    }
    fn prefix(&mut self, index: usize, loaded: bool, wrap: bool) -> R<Option<(usize, bool)>> {
        match self.source.prefix(index, loaded, wrap)? {
            Some(_) => Err(unsupported("of a line or wrap prefix")),
            None => Ok(None),
        }
    }
    fn prefix_elem(&self, _id: usize, _k: usize) -> Option<Elem> { None }
}

impl Lines for Hosted<'_, '_> {
    fn reseat(&mut self, start: i64) -> R<()> { self.reset(start) }
    fn line_start_before(&mut self, pos: i64) -> R<i64> { self.source.visible_line_start(pos) }
    fn begv(&self) -> i64 { self.source.begv() }
}
