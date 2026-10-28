//! Seam L: owned layout input. No evaluator, heap reference or glyph matrix.
use std::ops::Range;

/// GNU font-sort attributes. Height scoring remains deferred (DIVERGENCES.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontSelectionAttribute { Width, Height, Weight, Slant }
pub const DEFAULT_FONT_SELECTION_ORDER: [FontSelectionAttribute; 4] = [
    FontSelectionAttribute::Width, FontSelectionAttribute::Height,
    FontSelectionAttribute::Weight, FontSelectionAttribute::Slant,
];

/// Effective GNU font-sort metadata, including history omitted by the raw order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontSelectionState {
    order: [FontSelectionAttribute; 4],
    // Width, height, weight, slant. GNU font.c:2338-2354 overwrites only
    // mentioned fields, so duplicates take the last shift and omissions persist.
    shifts: [u8; 4],
    revision: u64,
}

impl Default for FontSelectionState {
    fn default() -> Self {
        Self { order: DEFAULT_FONT_SELECTION_ORDER, shifts: [23, 16, 9, 2], revision: 0 }
    }
}

impl FontSelectionState {
    pub fn order(self) -> [FontSelectionAttribute; 4] { self.order }
    pub fn shifts(self) -> [u8; 4] { self.shifts }
    pub fn revision(self) -> u64 { self.revision }

    /// Apply a validated order. A changed raw order frees realized answers in
    /// GNU xfaces.c:5899-5905, even if its effective shifts are unchanged.
    pub fn set_order(&mut self, order: [FontSelectionAttribute; 4]) -> bool {
        let changed = self.order != order;
        for (attribute, shift) in order.into_iter().zip([23, 16, 9, 2]) {
            let index = match attribute {
                FontSelectionAttribute::Width => 0,
                FontSelectionAttribute::Height => 1,
                FontSelectionAttribute::Weight => 2,
                FontSelectionAttribute::Slant => 3,
            };
            self.shifts[index] = shift;
        }
        self.order = order;
        if changed { self.revision = self.revision.wrapping_add(1); }
        changed
    }
}

/// Owned publication of GNU's alternative-font family alist. Validation and
/// family interning belong to the evaluator before publication.
/// Empty rows, duplicates and entry order are preserved, not normalized.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontAlternativeState {
    family_alist: Vec<Vec<String>>,
    revision: u64,
}

impl FontAlternativeState {
    pub fn family_alist(&self) -> &[Vec<String>] { &self.family_alist }
    pub fn revision(&self) -> u64 { self.revision }

    /// Every successful GNU setter frees realized faces, even for equal input.
    pub fn set_family_alist(&mut self, rows: Vec<Vec<String>>) {
        self.family_alist = rows;
        self.revision = self.revision.wrapping_add(1);
    }
}

/// One owned publication for the complete singleton frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameSnapshot {
    pub font_selection: FontSelectionState,
    pub font_alternatives: FontAlternativeState,
    pub width: u32,
    pub height: u32,
    pub windows: Vec<PlacedWindowSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedWindowSnapshot {
    pub x: u32,
    pub y: u32,
    pub snapshot: WindowSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceHandle(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub family: String,
    pub pixel_size: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceRun {
    pub bytes: Range<usize>,
    pub face: FaceHandle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSlice {
    pub utf8: String,
    pub face_runs: Vec<FaceRun>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowSnapshot {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub start_byte: usize,
    pub point_byte: usize,
    pub buffer_version: u64,
    pub text: TextSlice,
    pub mode_line: Option<TextSlice>,
    pub mode_line_face: FaceHandle,
    /// The header line above the text, painted in HEADER_LINE_FACE.
    pub header_line: Option<TextSlice>,
    pub header_line_face: FaceHandle,
    pub vscroll_px: u32,
    /// FaceHandle indexes this owned table; entry zero is the default face.
    pub faces: Vec<Face>,
}

/// One window's display input for a measurement: what the host would lay
/// out for these faces and text, without presenting it. Entry zero of
/// `faces` is the default face, as in `WindowSnapshot`.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasureInput {
    pub font_selection: FontSelectionState,
    pub font_alternatives: FontAlternativeState,
    pub faces: Vec<Face>,
    pub text: TextSlice,
}

/// The host's geometry for a `MeasureInput`, in whole pixels: each
/// character's advance and the row height its face gives it, in text
/// order (a newline advances 0), and each face's row height.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measured {
    pub glyphs: Vec<(u32, u32)>,
    pub face_heights: Vec<u32>,
}
