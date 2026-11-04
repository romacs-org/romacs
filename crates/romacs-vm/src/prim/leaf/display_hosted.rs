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
//! measures in the host's pixels. An escape or control sequence and a
//! display table's glyphs publish each glyph, in its own face, so the host
//! paints and measures what GNU shows; a glyphless character publishes a
//! stretch of the pixels GNU gives it (produce_glyphless_glyph), painted
//! blank where GNU draws its box. What the host does not draw as GNU does
//! (line and wrap prefixes) is refused: the host would not show what GNU
//! shows there.
use super::*;
use super::display_elements::{Source, View};
use super::display_motion::{Elem, Elements, Glyphless, Lines, Method, Shape, Slot, R};
use romacs_layout_input::StretchWidth;

/// The elements one host measurement covers past the first it needs.
const CHUNK: usize = 4096;

fn unsupported(what: &str) -> Fault { Fault::Error(format!("VM2 window-text-pixel-size {what} in a displayed window is unsupported")) }
/// Whether FAULT is such a refusal: what the display iterator cannot yet
/// measure, not an error of the caller's.
pub(super) fn refused(fault: &Fault) -> bool {
    matches!(fault, Fault::Error(m) if m.starts_with("VM2 window-text-pixel-size ") && m.ends_with(" is unsupported"))
}
fn not_as_gnu() -> Fault { unsupported("of text the host does not lay out as GNU displays it") }

/// The face a display element shows in: the buffer's at a position (GNU
/// face_at_buffer_position), or a string's own at STRPOS over that
/// (face_at_string_position, the buffer text's face as its base), over the
/// text's alone for an OVERLAY's string (face_for_overlay_string).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FaceKey { Buffer(i64),
    /// A string's character, or with GLYPH (else nil) a display vector
    /// glyph of it, in that glyph's face over the string's (merge_faces).
    String { string: Value, strpos: usize, charpos: i64, overlay: bool, glyph: Value },
    /// A display-table glyph's face over the buffer's (merge_faces).
    Glyph { face: Value, charpos: i64 } }

/// One element as a host shows it: its character, the face it shows in,
/// the buffer position it is at, what it is there and whether it REPLACES
/// the buffer's text there (the text itself, or what its `display' shows;
/// not an overlay string's), so point can show on it, and whether it is
/// word-wrap WHITESPACE (IT_DISPLAYING_WHITESPACE).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shown { pub c: char, pub face: FaceKey, pub charpos: i64, pub kind: ShownKind, pub replaces: bool, pub whitespace: bool }

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
    /// An image a placeholder space stands for.
    Image(romacs_layout_input::InlineImage),
}

/// How the host shows ELEM: None where it shows nothing of it (outside
/// the text area), else the character it paints, or `Err' where it does
/// not show what GNU does.
fn host_char(source: &mut Source<'_, '_>, e: &Elem) -> R<Option<Option<char>>> {
    if !e.text_area { return Ok(None); }
    let _ = source;
    Ok(Some(match (e.shape, e.method) {
        // A character, an escape's or a display table's glyph as it is.
        (Shape::Glyph { c, .. }, Method::Buffer | Method::String | Method::Vector { .. }) => char::from_u32(c),
        (Shape::Tab, Method::Buffer | Method::String | Method::Vector { .. }) => Some('\t'),
        // A glyphless character's box: a stretch of its width.
        (Shape::Stretch { .. } | Shape::Align { .. } | Shape::Glyphless { .. } | Shape::Image(..), _) => Some(' '),
        // Measured as the glyph it is relative to.
        (Shape::RelativeStretch { c, .. }, _) => char::from_u32(c),
        _ => None,
    }))
}

fn face_key(e: &Elem) -> FaceKey {
    if let Some(face) = e.ellipsis.filter(|&f| f != Value::NIL) { return FaceKey::Glyph { face, charpos: e.charpos }; }
    // A glyph's own face over the text's or the string's.
    if let Some(face) = e.face.filter(|_| e.object == Value::NIL) { return FaceKey::Glyph { face, charpos: e.charpos }; }
    match e.strpos {
        Some(strpos) if e.object != Value::NIL => FaceKey::String { string: e.object, strpos, charpos: e.charpos, overlay: !e.from_display,
            glyph: e.face.unwrap_or(Value::NIL) },
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

/// produce_glyphless_glyph: the pixels E, a glyphless character, takes
/// on a window-system frame: THIN_SPACE_WIDTH, its columns of the face's
/// base width for an empty box, or the wider of its code's (or acronym's)
/// two halves in `glyphless-char', with the box's bars and spaces, and no
/// less than the base width. Without a host, its columns.
fn glyphless_pixels(source: &mut Source<'_, '_>, e: &Elem) -> R<i64> {
    let Shape::Glyphless { width, c, how } = e.shape else { return Ok(0) };
    if !source.leaf().vm.has_layout_host() {
        let column = source.leaf().vm.window_state.column as i64;
        return Ok(width * column);
    }
    let window = source.window();
    let measure = |source: &mut Source<'_, '_>, text: &str, face: FaceKey| -> R<i64> {
        if text.is_empty() { return Ok(0); }
        let shown: Vec<(char, FaceKey)> = text.chars().map(|ch| (ch, face)).collect();
        let measured = source.leaf().vm.host_measure_elements(window, &shown).map_err(Fault::Error)?;
        Ok(measured.glyphs.iter().map(|&(w, _)| i64::from(w)).sum())
    };
    let base = measure(source, "x", face_key(e))?;
    let text = match how {
        Glyphless::ThinSpace => return Ok(1),
        Glyphless::EmptyBox => return Ok(base * source.leaf().character_width(c)?.clamp(1, 4)),
        Glyphless::HexCode => if c < 0x10000 { format!("{c:04X}") } else { format!("{c:06X}") },
        Glyphless::Acronym(string) => source.leaf().vm.str_view(string).map(|v| v.chars().map_while(char::from_u32).take_while(char::is_ascii).take(6).collect()).unwrap_or_default(),
    };
    let glyph = source.leaf().intern("glyphless-char");
    let face = FaceKey::Glyph { face: glyph, charpos: e.charpos };
    let upper = text.chars().count().div_ceil(2);
    let (top, bottom): (String, String) = (text.chars().take(upper).collect(), text.chars().skip(upper).collect());
    let boxed = measure(source, &top, face)?.max(measure(source, &bottom, face)?) + 4;
    Ok(boxed.max(base))
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
                None if let Shape::Image(image, _) = e.shape => (ShownKind::Image(image), e.object == Value::NIL),
                None if let Shape::Glyphless { .. } = e.shape => {
                    let width = glyphless_pixels(&mut source, &e)?;
                    (ShownKind::Stretch(StretchWidth::Width { width, zero_ok: false }), e.object == Value::NIL)
                }
                None if let Shape::RelativeStretch { factor, .. } = e.shape => (ShownKind::Relative { factor, c }, e.object == Value::NIL),
                None if e.object == Value::NIL && e.ellipsis.is_none() && e.method == Method::Buffer => (ShownKind::Buffer, true),
                // An escape's or display table's glyphs stand for the
                // buffer's character: point shows on the first.
                None if e.object == Value::NIL && e.ellipsis.is_none() => (ShownKind::Other { cursor: false }, true),
                None => {
                    let asks = match e.strpos { Some(k) if e.object != Value::NIL && !e.from_display =>
                        source.leaf().vm.property_textget(e.object, k as i64, cursor)? != Value::NIL, _ => false };
                    (ShownKind::Other { cursor: asks }, e.from_display)
                }
            };
            out.push(Shown { c, face: face_key(&e), charpos: e.charpos, kind, replaces, whitespace: e.whitespace });
        }
        Ok(out)
    }
}

pub(super) struct Hosted<'l, 'a> {
    source: Source<'l, 'a>,
    /// The host's advance and own row height for the elements from BASE
    /// on, by index since the last reset.
    base: usize,
    glyphs: Vec<(i64, i64, i64)>,
    /// The default face's row height, which every host row reaches, and
    /// its ascent.
    pub default_height: i64,
    pub default_ascent: i64,
}

impl<'l, 'a> Hosted<'l, 'a> {
    pub(super) fn new(source: Source<'l, 'a>) -> Self {
        Hosted { source, base: 0, glyphs: vec![], default_height: 0, default_ascent: 0 }
    }
    pub(super) fn source(&mut self) -> &mut Source<'l, 'a> { &mut self.source }
    /// Deliver from START, a line start, again.
    pub(super) fn reset(&mut self, start: i64) -> R<()> {
        (self.base, self.glyphs) = (0, vec![]);
        self.source.reset(start)
    }
    /// The host's advance and own height for element INDEX.
    pub(super) fn glyph(&mut self, index: usize) -> R<(i64, i64)> {
        self.glyph_rise(index).map(|(w, h, _)| (w, h))
    }
    /// The host's advance, own height and ascent for element INDEX.
    fn glyph_rise(&mut self, index: usize) -> R<(i64, i64, i64)> {
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
        self.glyphs = measured.glyphs.iter().zip(&measured.ascents).map(|(&(w, h), &a)| (i64::from(w), i64::from(h), i64::from(a))).collect();
        (self.default_height, self.default_ascent) = (i64::from(measured.face_heights[0]), i64::from(measured.face_ascents[0]));
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
        if c.is_none() { return Err(not_as_gnu()); }
        let (width, height, ascent) = self.glyph_rise(index)?;
        // Every host row reaches the default face's ascent and descent.
        let (ascent, descent) = (ascent.max(self.default_ascent), (height - ascent).max(self.default_height - self.default_ascent));
        let height = ascent + descent;
        let shape = match e.shape {
            Shape::Glyphless { .. } => Shape::PixelStretch {
                width: StretchWidth::Width { width: glyphless_pixels(&mut self.source, &e)?, zero_ok: false }, unnumbered: None, height, ascent },
            Shape::Glyph { c, .. } => Shape::Pixel { c, width, height, ascent },
            // Its own size, not the placeholder's.
            Shape::Image(image, spec) => Shape::Image(image, spec),
            Shape::Tab => Shape::PixelTab { space: width, height, ascent },
            // produce_stretch_glyph: NUMVAL (prop) * it2.pixel_width.
            Shape::RelativeStretch { factor, .. } => Shape::PixelStretch {
                width: StretchWidth::Width { width: (factor * width as f64) as i64, zero_ok: false }, unnumbered: None, height, ascent },
            shape => Shape::PixelStretch { width: stretch_width(shape).ok_or_else(not_as_gnu)?, unnumbered: e.unnumbered.and_then(stretch_width), height, ascent },
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
