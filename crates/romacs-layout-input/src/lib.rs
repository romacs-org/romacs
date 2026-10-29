//! Seam L: owned layout input. No evaluator, heap reference or glyph matrix.
use std::ops::Range;

/// GNU font-sort attributes. Height scoring remains deferred (DIVERGENCES.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontSelectionAttribute { Width, Height, Weight, Slant }
romacs_snapshot::snapshot_enum!(FontSelectionAttribute {
    FontSelectionAttribute::Width => (),
    FontSelectionAttribute::Height => (),
    FontSelectionAttribute::Weight => (),
    FontSelectionAttribute::Slant => (),
});
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
romacs_snapshot::snapshot_struct!(FontSelectionState { order, shifts, revision });

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
romacs_snapshot::snapshot_struct!(FontAlternativeState { family_alist, revision });

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

impl FrameSnapshot {
    /// Whether OTHER lays out to the same glyphs as this frame, cursors
    /// aside: equal but for publication versions and cursor visibility.
    pub fn lays_out_like(&self, other: &Self) -> bool {
        (self.font_selection, self.width, self.height) == (other.font_selection, other.width, other.height)
            && self.font_alternatives == other.font_alternatives
            && self.windows.len() == other.windows.len()
            && self.windows.iter().zip(&other.windows)
                .all(|(a, b)| (a.x, a.y) == (b.x, b.y) && a.snapshot.lays_out_like(&b.snapshot))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedWindowSnapshot {
    pub x: u32,
    pub y: u32,
    pub snapshot: WindowSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceHandle(pub u32);
romacs_snapshot::snapshot_tuple!(FaceHandle(f0));

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub family: String,
    pub pixel_size: f32,
}
romacs_snapshot::snapshot_struct!(Face { foreground, background, family, pixel_size });

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceRun {
    pub bytes: Range<usize>,
    pub face: FaceHandle,
}
romacs_snapshot::snapshot_struct!(FaceRun { bytes, face });

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSlice {
    pub utf8: String,
    pub face_runs: Vec<FaceRun>,
}
romacs_snapshot::snapshot_struct!(TextSlice { utf8, face_runs });

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
    pub vscroll_px: u32,
    /// FaceHandle indexes this owned table; entry zero is the default face.
    pub faces: Vec<Face>,
    /// The buffer's `tab-width', GNU's SANE_TAB_WIDTH (1 to 1000, else 8).
    pub tab_width: u32,
    /// How a line wider than the window shows.
    pub wrap: LineWrap,
    /// GNU's `w->cursor_off_p': `internal-show-cursor' hid this window's
    /// cursor, as blink-cursor-mode's timer does every other blink.
    pub cursor_off: bool,
}
romacs_snapshot::snapshot_struct!(WindowSnapshot { id, width, height, start_byte, point_byte, buffer_version, text, mode_line, mode_line_face, vscroll_px, faces, tab_width, wrap, cursor_off });

/// What laying a window out established about the text it published, in
/// byte offsets into that text: where its last displayed row ends (GNU's
/// `window_end_pos') and how far its fully visible rows reach, which is
/// where point can be without redisplay choosing another start.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowEnd {
    pub id: u64,
    /// The byte after the last displayed character.
    pub end: usize,
    /// Every position before this byte is on a fully visible row: one past
    /// the text's length when its end position is on one too.
    pub visible_end: usize,
    /// The text area's height: the window's less its mode line's.
    pub body_height: f32,
    /// The rows laid out from the window start, top to bottom, through the
    /// first that reaches the text area's bottom.
    pub rows: Vec<Row>,
}

/// One screen row: the byte it starts at, its top and its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub start: usize,
    pub y: f32,
    pub height: f32,
}

/// GNU's `it->line_wrap': `truncate-lines' (or a partial-width window
/// under `truncate-partial-width-windows') truncates, `word-wrap'
/// continues at word boundaries, else lines continue at the window edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineWrap { Truncate, #[default] Window, Word }
romacs_snapshot::snapshot_enum!(LineWrap {
    LineWrap::Truncate => (),
    LineWrap::Window => (),
    LineWrap::Word => (),
});

impl WindowSnapshot {
    /// Whether OTHER lays out to the same glyphs, cursor aside: equal but
    /// for `buffer_version' (the VM's conservative redraw signal, which also
    /// moves when nothing shown changed) and `cursor_off'.
    pub fn lays_out_like(&self, other: &Self) -> bool {
        // Exhaustive, so a new field must be placed on one side or the other.
        let WindowSnapshot { id, width, height, start_byte, point_byte, buffer_version: _,
            text, mode_line, mode_line_face, vscroll_px, faces, tab_width, wrap, cursor_off: _ } = self;
        (*id, *width, *height, *start_byte, *point_byte, *mode_line_face, *vscroll_px, *tab_width)
            == (other.id, other.width, other.height, other.start_byte, other.point_byte, other.mode_line_face, other.vscroll_px, other.tab_width)
            && *wrap == other.wrap && *text == other.text && *mode_line == other.mode_line && *faces == other.faces
    }
}

/// The stretch a TAB at X takes on a window-system frame (GNU xdisp.c
/// `gui_produce_glyphs`): to the next multiple of TAB_WIDTH spaces of the
/// tab's face, or to the one after when that is less than a space away. X
/// counts from the start of the line, in pixels.
pub fn tab_advance(x: f64, space: f64, tab_width: u32) -> f64 {
    let tab = f64::from(tab_width) * space;
    if tab <= 0.0 { return 0.0; }
    let mut next = ((x + tab) / tab).floor() * tab;
    if next - x < space { next += tab; }
    next - x
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
/// order (a newline advances 0, a TAB its face's space, from which
/// `tab_advance` makes its stretch), and each face's row height: the
/// default face's and those the text uses (0 for any other).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measured {
    pub glyphs: Vec<(u32, u32)>,
    pub face_heights: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::tab_advance;

    /// GNU's stops: the next multiple of TAB_WIDTH spaces, or the one
    /// after when that is less than a space away.
    #[test]
    fn tab_advance_reaches_the_next_stop_a_space_or_more_away() {
        assert_eq!(tab_advance(0.0, 10.0, 8), 80.0);
        assert_eq!(tab_advance(10.0, 10.0, 8), 70.0);
        assert_eq!(tab_advance(79.0, 10.0, 8), 81.0);
        assert_eq!(tab_advance(70.0, 10.0, 8), 10.0);
        assert_eq!(tab_advance(80.0, 10.0, 8), 80.0);
        assert_eq!(tab_advance(3.0, 7.0, 1), 11.0);
    }
}
