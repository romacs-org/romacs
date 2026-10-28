//! The display elements of a window a host lays out (VM2-WTPS1). The
//! host paints a window's buffer characters one glyph each, in the faces
//! its snapshot gives them, and breaks rows only at newlines. Where the
//! iterator's elements are exactly those characters, each becomes a
//! `Shape::Pixel` glyph with the advance and height the host's own layout
//! gives it, so the motion measures in the host's pixels. Any other
//! element (invisible or replaced text, overlay and prefix strings,
//! display-table glyphs, tabs and other escapes) is refused: the host
//! would not show what GNU shows there.
use super::*;
use super::display_elements::Source;
use super::display_motion::{Elem, Elements, Method, Shape, Slot, R};
use crate::coordinates::{ByteOffset, CharOffset};
use crate::data::Object;

/// The bytes one host measurement covers past the line it starts on.
const CHUNK: usize = 8192;

fn unsupported(what: &str) -> Fault { Fault::Error(format!("VM2 window-text-pixel-size {what} in a displayed window is unsupported")) }

pub(super) struct Hosted<'l, 'a> {
    source: Source<'l, 'a>,
    /// The position the elements start from (`Source::reset`).
    origin: i64,
    /// The characters from BASE on the host measured: advance, own row
    /// height, and whether the host paints them in the face GNU would.
    base: i64,
    glyphs: Vec<(i64, i64, bool)>,
    /// The default face's row height, which every host row reaches.
    pub default_height: i64,
}

impl<'l, 'a> Hosted<'l, 'a> {
    pub(super) fn new(source: Source<'l, 'a>) -> Self {
        Hosted { source, origin: 0, base: 0, glyphs: vec![], default_height: 0 }
    }
    pub(super) fn source(&mut self) -> &mut Source<'l, 'a> { &mut self.source }
    /// Deliver from START, a line start, again.
    pub(super) fn reset(&mut self, start: i64) -> R<()> {
        self.origin = start;
        self.source.reset(start)
    }
    /// The host's advance and own height for the character at POS.
    pub(super) fn glyph(&mut self, pos: i64) -> R<(i64, i64)> {
        if pos < self.base || pos >= self.base + self.glyphs.len() as i64 { self.measure(pos)?; }
        let (width, height, exact) = self.glyphs.get((pos - self.base) as usize).copied()
            .ok_or_else(|| Fault::Error("host measurement misses a character".into()))?;
        if !exact { return Err(unsupported("of a face the host does not paint")); }
        Ok((width, height))
    }
    /// Ask the host for the characters from POS's line start through
    /// the end of the line CHUNK bytes on, in the current buffer.
    fn measure(&mut self, pos: i64) -> R<()> {
        let vm = &mut *self.source.leaf().vm;
        let (range, base) = {
            let Some(Object::Buffer(b)) = vm.object_if(vm.slots[vm.buffer_slot].value) else {
                return Err(Fault::Error("text buffer required".into()));
            };
            let accessible = b.accessible();
            let text = b.as_str().as_bytes();
            let at = b.byte_of_char(CharOffset((pos - 1).max(0) as usize)).map_err(Fault::Error)?.0;
            let start = text[accessible.start.0..at].iter().rposition(|&c| c == b'\n').map_or(accessible.start.0, |i| accessible.start.0 + i + 1);
            let mut end = (at + CHUNK).min(accessible.end.0);
            while end < accessible.end.0 && !b.as_str().is_char_boundary(end) { end += 1; }
            let end = text[end..accessible.end.0].iter().position(|&c| c == b'\n').map_or(accessible.end.0, |i| end + i + 1);
            (start..end, b.char_of_byte(ByteOffset(start)).map_err(Fault::Error)?.0 as i64 + 1)
        };
        let (measured, exact) = vm.host_measure_buffer(range).map_err(Fault::Error)?;
        self.base = base;
        self.glyphs = measured.glyphs.into_iter().zip(exact).map(|((w, h), exact)| (i64::from(w), i64::from(h), exact)).collect();
        self.default_height = i64::from(measured.face_heights[0]);
        Ok(())
    }
    /// The default face's row height and the advance of C in it.
    pub(super) fn default_advance(&mut self, c: char) -> R<(i64, i64)> {
        let (advance, height) = self.source.leaf().vm.host_default_advance(c).map_err(Fault::Error)?;
        self.default_height = i64::from(height);
        Ok((i64::from(advance), i64::from(height)))
    }
    /// ELEM is the buffer's character at its position, the one after the
    /// element before INDEX's, as the host paints it.
    fn plain(&mut self, index: usize, e: &Elem) -> R<bool> {
        let Shape::Glyph { c, .. } = e.shape else { return Ok(false) };
        if e.method != Method::Buffer || !e.text_area || e.from_display || e.strpos.is_some() || e.object != Value::NIL
            || self.source.char_after(e.charpos)? != Some(c) {
            return Ok(false);
        }
        Ok(if index == 0 { e.charpos == self.origin }
            else { self.source.slot(index - 1)?.elem.is_some_and(|p| p.charpos + 1 == e.charpos) })
    }
}

impl Elements for Hosted<'_, '_> {
    fn slot(&mut self, index: usize) -> R<Slot> {
        let slot = self.source.slot(index)?;
        let Some(e) = slot.elem else { return Ok(slot) };
        if !self.plain(index, &e)? { return Err(unsupported("of text the host does not lay out as GNU displays it")); }
        let Shape::Glyph { c, .. } = e.shape else { unreachable!() };
        let (width, height) = self.glyph(e.charpos)?;
        let shape = Shape::Pixel { c, width, height: height.max(self.default_height) };
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
