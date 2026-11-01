//! One target's geometry: vertex streams per pipeline and the ordered
//! commands that draw them. Adjacent commands with the same pipeline,
//! binding and scissor merge, so a page of cards is a handful of draws while
//! paint order is kept exactly.

use super::draw::Source;
use crate::glyph_atlas::AnyAtlasEntry;
use crate::glyph_atlas::types::GlyphMaterialKind;
use crate::vertex::{GlyphVertex, RectVertex, RoundedRectVertex};
use romacs_display_protocol::font::GlyphSampling;
use std::ops::Range;

/// Device-pixel scissor `[x, y, width, height]`, never empty.
pub(super) type Scissor = [u32; 4];

#[derive(Clone, Copy, Debug)]
pub(super) enum Pipe {
    /// SDF rect terms from `rects`.
    Rect,
    /// A texture through the rect SDF, from `rects`.
    Sample(Source),
    /// Indexed path triangles from `fills`.
    Fill,
    /// Glyph quads from `glyphs`, bound to one atlas page.
    Glyph(AnyAtlasEntry),
}

impl Pipe {
    fn same_draw(self, other: Self) -> bool {
        match (self, other) {
            (Pipe::Rect, Pipe::Rect) | (Pipe::Fill, Pipe::Fill) => true,
            (Pipe::Sample(a), Pipe::Sample(b)) => a == b,
            (Pipe::Glyph(a), Pipe::Glyph(b)) => glyph_binding(a) == glyph_binding(b),
            _ => false,
        }
    }
}

pub(super) fn glyph_binding(entry: AnyAtlasEntry) -> (GlyphMaterialKind, u32, GlyphSampling) {
    entry.binding_id_value()
}

#[derive(Clone, Debug)]
pub(super) struct Cmd {
    pub pipe: Pipe,
    pub scissor: Scissor,
    /// Vertices (or indices, for `Fill`) of the pipe's stream.
    pub range: Range<u32>,
}

#[derive(Default)]
pub(super) struct Batch {
    pub rects: Vec<RoundedRectVertex>,
    pub fills: Vec<RectVertex>,
    pub fill_indices: Vec<u32>,
    pub glyphs: Vec<GlyphVertex>,
    pub cmds: Vec<Cmd>,
}

impl Batch {
    pub fn push_rects(&mut self, pipe: Pipe, scissor: Scissor, quads: Vec<RoundedRectVertex>) {
        let start = self.rects.len() as u32;
        self.rects.extend(quads);
        self.push(pipe, scissor, start..self.rects.len() as u32);
    }

    pub fn push_glyphs(&mut self, entry: AnyAtlasEntry, scissor: Scissor, quad: [GlyphVertex; 6]) {
        let start = self.glyphs.len() as u32;
        self.glyphs.extend(quad);
        self.push(Pipe::Glyph(entry), scissor, start..self.glyphs.len() as u32);
    }

    /// Record the indices appended to `fill_indices` since `start`.
    pub fn push_fill(&mut self, scissor: Scissor, start: u32) {
        self.push(Pipe::Fill, scissor, start..self.fill_indices.len() as u32);
    }

    fn push(&mut self, pipe: Pipe, scissor: Scissor, range: Range<u32>) {
        if range.is_empty() {
            return;
        }
        if let Some(last) = self.cmds.last_mut()
            && last.pipe.same_draw(pipe)
            && last.scissor == scissor
            && last.range.end == range.start
        {
            last.range.end = range.end;
            return;
        }
        self.cmds.push(Cmd {
            pipe,
            scissor,
            range,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_graph::draw::TargetId;

    fn quad() -> Vec<RoundedRectVertex> {
        vec![bytemuck::Zeroable::zeroed(); 6]
    }

    #[test]
    fn adjacent_draws_merge_but_paint_order_is_kept() {
        let full = [0, 0, 10, 10];
        let mut batch = Batch::default();
        batch.push_rects(Pipe::Rect, full, quad());
        batch.push_rects(Pipe::Rect, full, quad());
        batch.push_rects(Pipe::Sample(Source::Target(TargetId(3))), full, quad());
        batch.push_rects(Pipe::Rect, full, quad());
        batch.push_rects(Pipe::Rect, [0, 0, 5, 5], quad());
        let shape: Vec<_> = batch.cmds.iter().map(|c| c.range.clone()).collect();
        assert_eq!(shape, [0..12, 12..18, 18..24, 24..30]);
    }

    #[test]
    fn empty_ranges_record_nothing() {
        let mut batch = Batch::default();
        batch.push_rects(Pipe::Rect, [0, 0, 1, 1], Vec::new());
        batch.push_fill([0, 0, 1, 1], 0);
        assert!(batch.cmds.is_empty());
    }
}
