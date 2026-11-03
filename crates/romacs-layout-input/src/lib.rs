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
    /// The opacity of every background the frame paints, as its
    /// `alpha-background' setter last applied it (frame.c:6098).
    pub background_alpha: f32,
    /// Visible child frames painted over the windows, lowest first.
    pub children: Vec<ChildFrameSnapshot>,
    /// The tab bar above the windows, while `tab-bar-lines' shows one.
    pub tab_bar: Option<TabBarSnapshot>,
    /// How the panes are drawn around their windows' boxes.
    pub chrome: FrameChrome,
    /// The status bar between the windows and the mini-window, while
    /// `romacs-status-bar-lines' shows one.
    pub status_bar: Option<StatusBarSnapshot>,
}

/// The paint-only pane chrome of the root frame's windows but the
/// mini-window MINIBUFFER (`romacs-pane-*' frame parameters): corners of
/// RADIUS, a one-device-pixel hairline in BORDER round each window's box
/// and the window SELECTED's edge in FOCUS instead, and every other
/// window's colours blended DIM of the way toward the frame background.
/// INTERNAL_BORDER is the gap the windows are placed inside. The default
/// (classic) draws none of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameChrome {
    pub internal_border: u32,
    pub radius: u32,
    pub dim: f32,
    pub focus: Option<[u8; 3]>,
    pub border: Option<[u8; 3]>,
    pub selected: u64,
    pub minibuffer: u64,
}

impl FrameChrome {
    /// Whether it draws anything: GNU's look draws none of it.
    pub fn drawn(&self) -> bool {
        self.radius > 0 || self.dim > 0.0 || self.focus.is_some() || self.border.is_some()
    }
}

impl FrameSnapshot {
    /// Whether OTHER lays out to the same glyphs as this frame, cursors
    /// aside: equal but for publication versions and cursor visibility.
    /// Child frames lay out apart, so they do not count.
    pub fn lays_out_like(&self, other: &Self) -> bool {
        (self.font_selection, self.width, self.height) == (other.font_selection, other.width, other.height)
            && self.font_alternatives == other.font_alternatives
            && self.tab_bar == other.tab_bar
            && self.chrome == other.chrome
            && self.status_bar == other.status_bar
            && windows_lay_out_like(&self.windows, &other.windows)
    }
}

/// Whether window lists A and B lay out alike (`WindowSnapshot::lays_out_like').
pub fn windows_lay_out_like(a: &[PlacedWindowSnapshot], b: &[PlacedWindowSnapshot]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a.x, a.y) == (b.x, b.y) && a.snapshot.lays_out_like(&b.snapshot))
}

/// A child frame (GNU's `parent-frame'): its own windows in a rect over the
/// root frame's, in pixels. X and Y place its outer edge in the root frame;
/// the windows are placed inside it, within BORDER.
#[derive(Clone, Debug, PartialEq)]
pub struct ChildFrameSnapshot {
    /// The display's identity for the frame, never the root frame's 1.
    pub id: u64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub border: u32,
    pub border_color: [u8; 3],
    pub background: [u8; 3],
    /// Its own `alpha-background' (FrameSnapshot::background_alpha).
    pub background_alpha: f32,
    pub windows: Vec<PlacedWindowSnapshot>,
    /// How the host animates it: `romacs-layer-animation'.
    pub animation: LayerAnimation,
    /// The edge it is anchored at, its `romacs-layer-edge' parameter: it
    /// opens from there and grows away from it.
    pub edge: LayerEdge,
    /// The row of its first window that shows its selection, counted from
    /// the window start: its `romacs-layer-selection' parameter.
    pub selection: Option<u32>,
    /// Its corner radius in pixels (`romacs-layer-radius'), glow colour
    /// (the `romacs-layer-glow' face's foreground, for a layer with
    /// corners) and shadow (`romacs-layer-shadow').
    pub radius: u32,
    pub glow: Option<[u8; 3]>,
    pub shadow: bool,
}

/// `romacs-layer-animation': no layer animation, fades only, or fades,
/// slides and tweens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerAnimation { #[default] Off, Reduced, Full }

/// The edge a layer is anchored at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerEdge { #[default] Bottom, Top }

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedWindowSnapshot {
    pub x: u32,
    pub y: u32,
    pub snapshot: WindowSnapshot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    /// Byte offsets into `utf8` of characters that are raw bytes
    /// (GNU's eight-bit characters), shown as `\NNN`; sorted.
    pub raw: Vec<u32>,
}
romacs_snapshot::snapshot_struct!(TextSlice { utf8, face_runs, raw });
impl TextSlice {
    /// Whether the character at BYTE is a raw byte.
    pub fn is_raw(&self, byte: usize) -> bool {
        !self.raw.is_empty() && self.raw.binary_search(&(byte as u32)).is_ok()
    }
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
    /// How far the window is scrolled horizontally (GNU's `w->hscroll').
    pub hscroll: Hscroll,
    /// FaceHandle indexes this owned table; entry zero is the default face.
    pub faces: Vec<Face>,
    /// The buffer's `tab-width', GNU's SANE_TAB_WIDTH (1 to 1000, else 8).
    pub tab_width: u32,
    /// How a line wider than the window shows.
    pub wrap: LineWrap,
    /// GNU's `w->cursor_off_p': `internal-show-cursor' hid this window's
    /// cursor, as blink-cursor-mode's timer does every other blink.
    pub cursor_off: bool,
    /// The characters of TEXT shown as stretches, in byte order.
    pub stretches: Vec<Stretch>,
    /// `display-line-numbers': what starts each row.
    pub line_numbers: Option<LineNumbers>,
    /// The margins and fringes around the text area: WIDTH is the text
    /// area's, and the window's placed X its left edge.
    pub chrome: WindowChrome,
}
romacs_snapshot::snapshot_struct!(WindowSnapshot { id, width, height, start_byte, point_byte, buffer_version, text, mode_line, mode_line_face, header_line, header_line_face, vscroll_px, hscroll, faces, tab_width, wrap, cursor_off, stretches, line_numbers, chrome });

/// A window's horizontal scroll in pixels (GNU's first_visible_x): the x
/// on each line, from its start after any line number, that shows at the
/// text area's left edge. ROWS applies to every row but point's, which
/// shows from POINT_ROW: they differ only under `auto-hscroll-mode'
/// `current-line', where the other rows keep `min_hscroll' (init_iterator,
/// display_line's hscroll_this_line). Scrolled rows are truncated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hscroll {
    pub rows: u32,
    pub point_row: u32,
}
romacs_snapshot::snapshot_struct!(Hscroll { rows, point_row });

impl Hscroll {
    /// The same scroll for every row.
    pub fn uniform(px: u32) -> Self { Self { rows: px, point_row: px } }
    pub fn is_zero(&self) -> bool { self.rows == 0 && self.point_row == 0 }
}

/// What GNU's window box shows of a window beside its text area
/// (window_box, dispextern.h): its margins and fringes in pixels, the
/// fringes between the margins and the text or, when OUTSIDE_MARGINS,
/// outside the margins, painted in FRINGE_FACE; the vertical border GNU
/// draws at a window's left edge when another window is left of it
/// (gui_draw_vertical_border), in BORDER's foreground; the bitmaps the
/// fringes show; and the window's right and bottom dividers (window.h:707),
/// right of and below its box, outside the snapshot's width and height. A
/// window with neither margins nor fringes has the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowChrome {
    pub left_margin: u32,
    pub right_margin: u32,
    pub left_fringe: u32,
    pub right_fringe: u32,
    pub outside_margins: bool,
    pub fringe_face: FaceHandle,
    pub border: Option<FaceHandle>,
    pub indicators: Indicators,
    pub right_divider: u32,
    pub bottom_divider: u32,
}
romacs_snapshot::snapshot_struct!(WindowChrome { left_margin, right_margin, left_fringe, right_fringe, outside_margins, fringe_face, border, indicators, right_divider, bottom_divider });

impl WindowChrome {
    /// The pixels left and right of the text area inside the window's box:
    /// its margins and fringes, not its divider.
    pub fn insets(&self) -> (u32, u32) {
        (self.left_margin + self.left_fringe, self.right_margin + self.right_fringe)
    }
}

/// The bitmaps GNU's fringes show for each logical indicator
/// (fringe-indicator-alist, update_window_fringes), [left, right]: none
/// where that side shows none. BOB and EOB say whether the window's text
/// starts at the buffer's accessible start and reaches its end, which the
/// boundary indicators mark.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Indicators {
    pub truncation: [Option<FringeBitmap>; 2],
    pub continuation: [Option<FringeBitmap>; 2],
    pub top: [Option<FringeBitmap>; 2],
    pub bottom: [Option<FringeBitmap>; 2],
    pub top_bottom: [Option<FringeBitmap>; 2],
    pub up: [Option<FringeBitmap>; 2],
    pub down: [Option<FringeBitmap>; 2],
    pub empty_line: [Option<FringeBitmap>; 2],
    pub bob: bool,
    pub eob: bool,
}
romacs_snapshot::snapshot_struct!(Indicators { truncation, continuation, top, bottom, top_bottom, up, down, empty_line, bob, eob });

/// One fringe bitmap (fringe.c `struct fringe_bitmap`): a row of BITS per
/// pixel row, its leftmost of WIDTH pixels in the most significant bit,
/// drawn centred in its fringe in FACE's foreground, placed in its row as
/// ALIGN says and, with a PERIOD, repeated down the whole row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FringeBitmap {
    /// GNU's bitmap number (the symbol's `fringe' property).
    pub number: u16,
    pub bits: Vec<u16>,
    pub width: u8,
    pub align: BitmapAlign,
    pub period: u8,
    pub face: FaceHandle,
}
romacs_snapshot::snapshot_struct!(FringeBitmap { number, bits, width, align, period, face });

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BitmapAlign { #[default] Center, Top, Bottom }
romacs_snapshot::snapshot_enum!(BitmapAlign {
    BitmapAlign::Center => (),
    BitmapAlign::Top => (),
    BitmapAlign::Bottom => (),
});

/// The frame's tab bar (GNU's tab bar window, xdisp.c display_tab_bar):
/// one row of TEXT across the top of the frame, HEIGHT pixels tall, on
/// FACE's background, its faces indexing FACES.
#[derive(Clone, Debug, PartialEq)]
pub struct TabBarSnapshot {
    pub height: u32,
    pub text: TextSlice,
    pub face: FaceHandle,
    pub faces: Vec<Face>,
}

/// The root frame's status bar (VM2-STATUS-BAR): one row of TEXT, the
/// expanded mode line of WINDOW (the window GNU's mode-line face rule
/// treats as selected), HEIGHT pixels tall at X, Y and WIDTH wide in the
/// frame, on FACE's background, its faces indexing FACES. FOCUSED says
/// whether WINDOW holds the input focus rather than a child frame's
/// window.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusBarSnapshot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub text: TextSlice,
    pub face: FaceHandle,
    pub faces: Vec<Face>,
    pub window: u64,
    pub focused: bool,
}

/// GNU's line numbers (xdisp.c maybe_produce_line_number): every row
/// starts with the label of the line starting at its byte, in FACE
/// (CURRENT for point's line), else as many blanks as its line's label
/// has glyphs (COLUMNS before any label); a TAB stop and `:align-to'
/// count from after them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineNumbers {
    pub columns: u32,
    pub face: FaceHandle,
    pub current: FaceHandle,
    /// (byte of a line start in the text, its label, whether point's line).
    pub labels: Vec<(usize, String, bool)>,
}
romacs_snapshot::snapshot_struct!(LineNumbers { columns, face, current, labels });

/// A `(space ...)' display specification the host lays out in place of
/// the character of a window's text at BYTE (a placeholder space in the
/// stretch's face, which gives its height and background).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stretch {
    pub byte: usize,
    pub width: StretchWidth,
}
romacs_snapshot::snapshot_struct!(Stretch { byte, width });

/// A stretch's width before it depends on where it is laid out, in pixels
/// (GNU's produce_stretch_glyph): `:width', which may be zero when ZERO_OK,
/// or `:align-to' TARGET pixels from the text area's left edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StretchWidth { Width { width: i64, zero_ok: bool }, AlignTo(i64) }
romacs_snapshot::snapshot_enum!(StretchWidth {
    StretchWidth::Width { width, zero_ok } => (width, zero_ok),
    StretchWidth::AlignTo(target) => (target),
});

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
    /// The text area's height: the window's less its mode and header lines'.
    pub body_height: f32,
    /// The rows laid out from the window start, top to bottom, through the
    /// first that reaches the text area's bottom.
    pub rows: Vec<Row>,
    /// Where point shows, when a laid-out row holds it.
    pub point: Option<PointPlace>,
}

/// Point on its row, in pixels from the text area's left edge, as GNU's
/// hscroll_window_tree reads the cursor: X, where the cursor is drawn,
/// which a truncated row keeps inside the text area (set_cursor_from_row
/// puts it on the glyph past the row's last position); LINE_X, where point
/// is on its line laid out unscrolled and infinitely wide, from the
/// line's start; GUTTER, the line number's width before the text; whether
/// the row is TRUNCATED on the LEFT (scrolled) or RIGHT; and whether point
/// is AT_EOL, on its line's newline or the text's end.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointPlace {
    pub x: f32,
    pub line_x: f32,
    pub gutter: f32,
    pub truncated_left: bool,
    pub truncated_right: bool,
    pub at_eol: bool,
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
            text, mode_line, mode_line_face, header_line, header_line_face, vscroll_px, hscroll, faces, tab_width, wrap, cursor_off: _, stretches, line_numbers, chrome } = self;
        (*id, *width, *height, *start_byte, *point_byte, *mode_line_face, *header_line_face, *vscroll_px, *hscroll, *tab_width)
            == (other.id, other.width, other.height, other.start_byte, other.point_byte, other.mode_line_face, other.header_line_face, other.vscroll_px, other.hscroll, other.tab_width)
            && *wrap == other.wrap && *stretches == other.stretches && *line_numbers == other.line_numbers && *text == other.text && *mode_line == other.mode_line && *header_line == other.header_line && *faces == other.faces && *chrome == other.chrome
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

/// The pixels a stretch of WIDTH at X on its row takes (GNU xdisp.c
/// `produce_stretch_glyph` on a window-system frame), LINE_X being X
/// counted from the start of the line, as `:align-to' measures. A width
/// that is not positive is one pixel unless ZERO_OK; on a row that
/// continues (LAST_VISIBLE_X given), a stretch that would pass its last
/// visible x ends a pixel before it, so it never continues the row. The
/// answer is a single glyph's when positive, and none otherwise.
pub fn stretch_advance(width: StretchWidth, x: f64, line_x: f64, last_visible_x: Option<f64>) -> f64 {
    let (mut width, zero_ok) = match width {
        StretchWidth::Width { width, zero_ok } => (width as f64, zero_ok),
        StretchWidth::AlignTo(target) => ((target as f64 - line_x).max(0.0), true),
    };
    if width <= 0.0 && (width < 0.0 || !zero_ok) { width = 1.0; }
    if let Some(last) = last_visible_x && width > 0.0 && x + width > last {
        width = last - x - 1.0;
    }
    width
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
    use super::{stretch_advance, tab_advance, StretchWidth};

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

    /// GNU's stretch widths: `:width' as given, at least a pixel unless
    /// zero is allowed; `:align-to' to its target from the line's start,
    /// never negative; on a continued row, a pixel short of the last
    /// visible x.
    #[test]
    fn stretch_advance_is_produce_stretch_glyphs() {
        let width = |width, zero_ok| StretchWidth::Width { width, zero_ok };
        assert_eq!(stretch_advance(width(20, true), 10.0, 10.0, None), 20.0);
        assert_eq!(stretch_advance(width(0, true), 10.0, 10.0, None), 0.0);
        assert_eq!(stretch_advance(width(0, false), 10.0, 10.0, None), 1.0);
        assert_eq!(stretch_advance(width(-3, true), 10.0, 10.0, None), 1.0);
        assert_eq!(stretch_advance(StretchWidth::AlignTo(50), 10.0, 30.0, None), 20.0);
        assert_eq!(stretch_advance(StretchWidth::AlignTo(20), 10.0, 30.0, None), 0.0);
        assert_eq!(stretch_advance(width(500, true), 10.0, 10.0, Some(100.0)), 89.0);
        assert_eq!(stretch_advance(width(500, true), 10.0, 10.0, None), 500.0);
        assert_eq!(stretch_advance(StretchWidth::AlignTo(500), 0.0, 0.0, Some(100.0)), 99.0);
    }
}
