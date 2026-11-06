//! Seam L: owned layout input. No evaluator, heap reference or glyph matrix.
use std::ops::Range;
pub mod cluster;
pub mod fontset;
pub use fontset::{Alternative, Alternatives, FontDef, Fontset, FontsetMode, FontsetState, FontsetTarget, OpenedDefFont, OtfSpec};

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
    /// The fontsets characters resolve through.
    pub fontsets: std::sync::Arc<FontsetState>,
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
    /// How a character no font has a glyph for shows.
    pub no_font: NoFont,
}

/// How a window-system frame shows a character no installed font has a
/// glyph for: `glyphless-char-display''s `no-font' extra slot, as
/// lookup_glyphless_char_display reads it for C -1 (the graphical half of
/// a cons; nil, `zero-width' or an unknown method is `empty-box').
/// `hex-code' is GNU's default (`glyphless-char-display-control').
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NoFont { ThinSpace, EmptyBox, #[default] HexCode, Acronym(String) }
romacs_snapshot::snapshot_enum!(NoFont {
    NoFont::ThinSpace => (),
    NoFont::EmptyBox => (),
    NoFont::HexCode => (),
    NoFont::Acronym(text) => (text),
});

/// The paint-only pane chrome of the root frame's windows but the
/// mini-window MINIBUFFER (`romacs-pane-*' frame parameters): corners of
/// RADIUS, a one-device-pixel hairline in BORDER round each window's box
/// and the window SELECTED's edge in FOCUS instead, and every other
/// window's colours blended DIM of the way toward the frame background.
/// INTERNAL_BORDER is the gap the windows are placed inside. The default
/// (classic) draws none of it. CURSOR is the frame's `cursor-color',
/// which every frame shows, classic ones too.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameChrome {
    pub internal_border: u32,
    pub radius: u32,
    pub dim: f32,
    pub focus: Option<[u8; 3]>,
    pub border: Option<[u8; 3]>,
    pub cursor: Option<[u8; 3]>,
    pub selected: u64,
    pub minibuffer: u64,
    pub style: Style,
}

/// The material a frame's layers are drawn in, from its `romacs-style'
/// parameter (rlisp/romacs-chrome.el); the default is the `flat' style.
/// LAYER_RADIUS is the corners of a layer that names none, BAND_RADIUS of a
/// picker's selected row; SHADOW is a layer's (reach, drop, opacity), GLOW
/// its (reach, opacity), in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub layer_radius: f32,
    pub band_radius: f32,
    pub shadow: [f32; 3],
    pub glow: [f32; 2],
}

impl Default for Style {
    fn default() -> Self {
        Style { layer_radius: 8.0, band_radius: 4.0, shadow: [18.0, 6.0, 0.55], glow: [14.0, 0.35] }
    }
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
            && self.fontsets == other.fontsets
            && self.tab_bar == other.tab_bar
            && self.chrome == other.chrome
            && self.status_bar == other.status_bar
            && self.no_font == other.no_font
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
    /// A 1px hairline round it in this colour, drawn even on a glowing
    /// layer: the foreground of the face its `romacs-layer-outline-face'
    /// parameter names (an error toast's).
    pub outline: Option<[u8; 3]>,
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
    /// CSS weight, 400 normal and 700 bold (`:weight').
    pub weight: u16,
    /// Italic or oblique (`:slant').
    pub italic: bool,
    /// OS/2 width class, 5 normal (`:width').
    pub width: u16,
    /// The lines the face draws over its text (`:underline', `:overline',
    /// `:strike-through').
    pub decor: Decor,
}
romacs_snapshot::snapshot_struct!(Face { foreground, background, family, pixel_size, weight, italic, width, decor });

/// A face's lines, GNU's `:underline', `:overline' and `:strike-through'.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decor {
    pub underline: Option<Line>,
    pub overline: Option<Line>,
    pub strike: Option<Line>,
    /// GNU's `:box'.
    pub boxed: Option<BoxLine>,
}
romacs_snapshot::snapshot_struct!(Decor { underline, overline, strike, boxed });

/// A face's box (xfaces.c realize_gui_face): VERTICAL the left and right
/// edges' width, HORIZONTAL the top and bottom edges', in device pixels,
/// each drawn outside the text when positive (widening the run and the
/// row) and inside it when negative; COLOR, else the foreground; STYLE 1 a
/// line, 2 a released button, 3 a pressed one (protocol `BoxType' codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxLine {
    pub vertical: i16,
    pub horizontal: i16,
    pub color: Option<[u8; 3]>,
    pub style: u8,
}
romacs_snapshot::snapshot_struct!(BoxLine { vertical, horizontal, color, style });

/// One line a face draws: STYLE is romacs-display-protocol's
/// `UnderlineStyle' code (1 line, 2 double, 3 wave, 4 dotted, 5 dashed;
/// overlines and strike-throughs are 1), COLOR the line's own colour, else
/// the face's foreground; for an underline POSITION, `:position' (at the
/// descent line, that many pixels above it), else where the font says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub style: u8,
    pub color: Option<[u8; 3]>,
    pub position: Option<u16>,
}
romacs_snapshot::snapshot_struct!(Line { style, color, position });

/// Where a window's underlines go (xterm.c x_draw_glyph_string): its
/// buffer's `underline-minimum-offset', `x-underline-at-descent-line' and
/// `x-use-underline-position-properties'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnderlineRules {
    pub minimum_offset: u32,
    pub at_descent_line: bool,
    pub use_position: bool,
}
romacs_snapshot::snapshot_struct!(UnderlineRules { minimum_offset, at_descent_line, use_position });

impl Default for UnderlineRules {
    fn default() -> Self { Self { minimum_offset: 1, at_descent_line: false, use_position: true } }
}

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
    /// The newlines whose row paints past its text to the window's edge,
    /// each with the face it paints in; sorted by byte.
    pub extends: Vec<Extend>,
}
romacs_snapshot::snapshot_struct!(TextSlice { utf8, face_runs, raw, extends });

/// xdisp.c extend_face_to_end_of_line's face for the row the newline at
/// BYTE of a text ends: the merge of only those faces there whose
/// `:extend' is non-nil.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extend {
    pub byte: u32,
    pub face: FaceHandle,
}
romacs_snapshot::snapshot_struct!(Extend { byte, face });
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
    /// The characters of TEXT shown as images, in byte order.
    pub images: Vec<InlineImage>,
    /// The bytes of TEXT whose character's word-wrap class is not its own,
    /// sorted: a space or TAB there is no wrap whitespace, another
    /// character is. GNU's IT_DISPLAYING_WHITESPACE looks under a stretch
    /// or a string at the buffer's text.
    pub wrap_flips: Vec<u32>,
    /// `display-line-numbers': what starts each row.
    pub line_numbers: Option<LineNumbers>,
    /// The margins and fringes around the text area: WIDTH is the text
    /// area's, and the window's placed X its left edge.
    pub chrome: WindowChrome,
    /// Where its underlines go.
    pub underline_rules: UnderlineRules,
    /// The characters of the header line and of the mode line shown as
    /// stretches, in byte order: an `:align-to' target counts from the
    /// text area's left edge, as GNU's display of a band measures it.
    pub band_stretches: BandStretches,
}
romacs_snapshot::snapshot_struct!(WindowSnapshot { id, width, height, start_byte, point_byte, buffer_version, text, mode_line, mode_line_face, header_line, header_line_face, vscroll_px, hscroll, faces, tab_width, wrap, cursor_off, stretches, images, wrap_flips, line_numbers, chrome, underline_rules, band_stretches });

/// A window's header-line and mode-line stretches (`WindowSnapshot').
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BandStretches {
    pub header_line: Vec<Stretch>,
    pub mode_line: Vec<Stretch>,
}
romacs_snapshot::snapshot_struct!(BandStretches { header_line, mode_line });

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
    /// The face the fringes' background is painted in.
    pub fringe_face: FaceHandle,
    pub border: Option<FaceHandle>,
    pub indicators: Indicators,
    pub right_divider: u32,
    pub bottom_divider: u32,
    /// The display table's truncation and continuation glyphs (disptab.h
    /// DISP_TRUNC_GLYPH, DISP_CONTINUE_GLYPH), where it sets them to a
    /// character without a face: shown in the default face for the `$' and
    /// `\\' a window without a fringe shows.
    pub line_glyphs: [Option<char>; 2],
}
romacs_snapshot::snapshot_struct!(WindowChrome { left_margin, right_margin, left_fringe, right_fringe, outside_margins, fringe_face, border, indicators, right_divider, bottom_divider, line_glyphs });

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
    /// Its segments and icons.
    pub marks: BandMarks,
}
romacs_snapshot::snapshot_struct!(StatusBarSnapshot { x, y, width, height, text, face, faces, window, focused, marks });

/// What the status bar's painter reads of its text besides faces
/// (VM2-STATUS-BAR): the segments it is made of, each a pill on its own
/// background or not, and the characters drawn as icons (byte, the icon's
/// character). A segment named `right-align' splits the bar: what comes
/// after it is laid out from the right edge.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BandMarks {
    pub segments: Vec<Segment>,
    pub icons: Vec<(usize, char)>,
}
romacs_snapshot::snapshot_struct!(BandMarks { segments, icons });

/// One segment of a band: its bytes, its name, and its pill.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub bytes: Range<usize>,
    pub name: String,
    pub pill: Option<Pill>,
}
romacs_snapshot::snapshot_struct!(Segment { bytes, name, pill });

/// A rounded background behind a segment's text: its colour, corner
/// radius in pixels, and the colour of the text on it where the pill
/// gives its own (`romacs-pill-text').
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pill {
    pub background: [u8; 3],
    pub radius: u32,
    pub foreground: Option<[u8; 3]>,
}
romacs_snapshot::snapshot_struct!(Pill { background, radius, foreground });

impl BandMarks {
    /// The bytes of the `right-align' marker, where the band splits.
    pub fn right_align(&self) -> Option<Range<usize>> { self.marker("right-align") }

    /// The bytes of the `centre-align' marker, where the band's centred zone
    /// starts.
    pub fn centre(&self) -> Option<Range<usize>> { self.marker("centre-align") }

    fn marker(&self, name: &str) -> Option<Range<usize>> {
        self.segments.iter().find(|s| s.name == name).map(|s| s.bytes.clone())
    }

    /// TEXT's characters by zone, as character index ranges: the left
    /// zone, the `centre-align' marker's, the centred zone, the `right-align'
    /// marker's, the right zone. A missing marker is empty, and so is the
    /// zone it starts.
    pub fn zones(&self, text: &TextSlice) -> [Range<usize>; 5] {
        let starts: Vec<usize> = text.utf8.char_indices().map(|(at, _)| at).collect();
        let n = starts.len();
        let span = |bytes: Option<Range<usize>>| bytes.map(|b| starts.partition_point(|&at| at < b.start)..starts.partition_point(|&at| at < b.end));
        let right = span(self.right_align()).unwrap_or(n..n);
        let centre = span(self.centre()).filter(|c| c.end <= right.start).unwrap_or(right.start..right.start);
        [0..centre.start, centre.clone(), centre.end..right.start, right.clone(), right.end..n]
    }
}

/// The width of the status bar's accent block at its left edge, which no
/// character covers.
pub const STATUS_BAR_ACCENT: f32 = 3.0;

/// The status bar's room at either end, left of its first character and
/// right of its last (r0 review: 12 px outer padding).
pub const STATUS_BAR_PAD: f32 = 12.0;

/// The least room between the centred zone and a side zone; a side that
/// would come closer pushes the centre away (modeline.md §14.2).
pub const STATUS_BAR_ZONE_GAP: f32 = 24.0;

impl StatusBarSnapshot {
    /// Where each character goes (`BandMarks::places'), inside the bar's
    /// padding. The painter (layout2) and `romacs-status-bar-string' (the
    /// VM) both place by this.
    pub fn places(&self, advances: &[f32]) -> Vec<Option<(f32, f32)>> {
        self.marks.places(&self.text, self.width as f32, STATUS_BAR_PAD, advances)
    }

    /// What the bar shows (`BandMarks::shown_text').
    pub fn shown_text(&self, places: &[Option<(f32, f32)>], column: f32) -> String {
        self.marks.shown_text(&self.text, places, column)
    }

    /// The characters the bar fits in COLUMN-wide cells: its width less
    /// its padding.
    pub fn columns(width: u32, column: u32) -> usize {
        ((width as f32 - 2.0 * STATUS_BAR_PAD).max(0.0) / column.max(1) as f32) as usize
    }
}

impl BandMarks {
    /// Where each character of TEXT, a marked band WIDTH wide with PAD of
    /// room at either end, goes in text order, given ADVANCES, their
    /// advances: its x from the band's left edge and its advance, or None
    /// where the band clips it (modeline.md §14). The right zone, after
    /// the `right-align' marker, is flush with the right end and shows
    /// whole when it fits. The centred zone, between the `centre-align' marker
    /// and that one, is centred on the band and held at least
    /// `STATUS_BAR_ZONE_GAP' from either side, pushed as little as that
    /// needs; with no room for both gaps it follows the left zone, both
    /// clipped where the right zone starts. Each zone shows a prefix of
    /// its characters, and a marker's characters span the room before the
    /// zone it starts.
    pub fn places(&self, text: &TextSlice, width: f32, pad: f32, advances: &[f32]) -> Vec<Option<(f32, f32)>> {
        let inner = (width - 2.0 * pad).max(0.0);
        let [left, centre_marker, centre, right_marker, right] = self.zones(text);
        let sum = |range: &Range<usize>| -> f32 { advances[range.clone()].iter().sum() };
        let (left_w, centre_w, right_w) = (sum(&left), sum(&centre), sum(&right));
        let right_x = if right_marker.is_empty() { inner } else { (inner - right_w).max(0.0) };
        // The centred zone's start: centred on the whole band, then pushed
        // off either side; where both push, after the left zone.
        let centre_x = (!centre_marker.is_empty()).then(|| {
            let lo = if left_w > 0.0 { left_w + STATUS_BAR_ZONE_GAP } else { 0.0 };
            let hi = if right_w > 0.0 { right_x - STATUS_BAR_ZONE_GAP } else { inner } - centre_w;
            let ideal = width / 2.0 - pad - centre_w / 2.0;
            if lo <= hi { ideal.clamp(lo, hi) } else { left_w }
        });
        let mut places: Vec<Option<(f32, f32)>> = Vec::with_capacity(advances.len());
        let run = |range: Range<usize>, from: f32, limit: f32, places: &mut Vec<Option<(f32, f32)>>| -> f32 {
            let mut x = from;
            let mut end = from;
            for &advance in &advances[range] {
                let fits = x + advance <= limit + 0.001 && end == x;
                places.push(fits.then_some((x, advance)));
                if fits { end = x + advance; }
                x += advance;
            }
            end
        };
        let room = |range: Range<usize>, from: f32, to: f32, places: &mut Vec<Option<(f32, f32)>>| {
            for _ in range { places.push(Some((from, (to - from).max(0.0)))); }
        };
        let left_end = run(left, 0.0, right_x, &mut places);
        let mut end = left_end;
        if let Some(x) = centre_x {
            // The centre marker's room reaches the centred zone where that
            // shows; where it shows nothing, the right marker's room is all.
            let x = x.max(left_end);
            let marker = places.len()..places.len() + centre_marker.len();
            room(centre_marker, left_end, left_end, &mut places);
            let centre_end = run(centre, x, right_x, &mut places);
            if centre_end > x {
                places[marker].fill(Some((left_end, x - left_end)));
                end = centre_end;
            }
        }
        if !right_marker.is_empty() {
            room(right_marker, end, right_x, &mut places);
            run(right, right_x, inner, &mut places);
        }
        places.into_iter().map(|place| place.map(|(x, w)| (x + pad, w))).collect()
    }

    /// What a marked band of TEXT shows, placed by PLACES (`places'):
    /// each shown character, an icon's character where it is drawn as
    /// one, and for each marker's room as many spaces as COLUMN-wide cells
    /// fill it.
    pub fn shown_text(&self, text: &TextSlice, places: &[Option<(f32, f32)>], column: f32) -> String {
        let [_, centre_marker, _, right_marker, _] = self.zones(text);
        let mut icons = self.icons.iter().peekable();
        let mut shown = String::new();
        for (i, ((byte, ch), place)) in text.utf8.char_indices().zip(places).enumerate() {
            while icons.peek().is_some_and(|(at, _)| *at < byte) { icons.next(); }
            let Some((_, width)) = place else { continue };
            if centre_marker.contains(&i) || right_marker.contains(&i) {
                if i == centre_marker.start || i == right_marker.start {
                    shown.extend(std::iter::repeat_n(' ', (width / column.max(1.0)).floor() as usize));
                }
                continue;
            }
            shown.push(icons.peek().filter(|(at, _)| *at == byte).map_or(ch, |&&(_, icon)| icon));
        }
        shown
    }

    /// Whether the band has anything a marked painter draws.
    pub fn is_empty(&self) -> bool { self.segments.is_empty() && self.icons.is_empty() }
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

/// An `(image ...)' display specification the host lays out in place of
/// the character of a window's text at BYTE (a placeholder space in the
/// image's face), as GNU's produce_image_glyph makes its glyph: a slot
/// WIDTH pixels wide, ASCENT above the row's baseline and DESCENT below
/// it, holding the slice SOURCE (x, y, width, height in the image's own
/// pixels) of the image FULL pixels in size, IMAGE in the renderer's
/// cache, at BODY_X and BODY_Y in the slot (its margins and relief).
/// RELIEF is `:relief' (raised when positive), its colors from
/// RELIEF_BACKGROUND (0xRRGGBB), else the face's background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlineImage {
    pub byte: usize,
    pub image: u32,
    pub width: i64,
    pub ascent: i64,
    pub descent: i64,
    pub body_x: i64,
    pub body_y: i64,
    pub source: (i64, i64, i64, i64),
    pub full: (i64, i64),
    pub relief: i64,
    pub relief_background: Option<u32>,
}
romacs_snapshot::snapshot_struct!(InlineImage { byte, image, width, ascent, descent, body_x, body_y, source, full, relief, relief_background });

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
            text, mode_line, mode_line_face, header_line, header_line_face, vscroll_px, hscroll, faces, tab_width, wrap, cursor_off: _, stretches, images, wrap_flips, line_numbers, chrome, underline_rules, band_stretches } = self;
        (*id, *width, *height, *start_byte, *point_byte, *mode_line_face, *header_line_face, *vscroll_px, *hscroll, *tab_width)
            == (other.id, other.width, other.height, other.start_byte, other.point_byte, other.mode_line_face, other.header_line_face, other.vscroll_px, other.hscroll, other.tab_width)
            && *wrap == other.wrap && *stretches == other.stretches && *images == other.images && *wrap_flips == other.wrap_flips && *line_numbers == other.line_numbers && *text == other.text && *mode_line == other.mode_line && *header_line == other.header_line && *faces == other.faces && *chrome == other.chrome
            && *underline_rules == other.underline_rules && *band_stretches == other.band_stretches
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
    pub fontsets: std::sync::Arc<FontsetState>,
    pub faces: Vec<Face>,
    pub text: TextSlice,
    /// How a character no font has a glyph for shows, as the frame's.
    pub no_font: NoFont,
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
    /// Each glyph's ascent, its face's: where the row's baseline is below
    /// its top when it is the tallest above the baseline.
    pub ascents: Vec<u32>,
    /// Each face's ascent, beside FACE_HEIGHTS.
    pub face_ascents: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::{stretch_advance, tab_advance, BandMarks, Segment, StretchWidth, TextSlice, STATUS_BAR_ZONE_GAP};

    /// A band of TEXT with a `centre-align' marker at byte C and a `right-align'
    /// one at byte R (each one character), placed WIDTH wide with 12 px
    /// padding, a 10 px advance each.
    fn zoned(text: &str, c: usize, r: usize, width: f32) -> Vec<Option<(f32, f32)>> {
        let marker = |at: usize, name: &str| Segment { bytes: at..at + 1, name: name.into(), pill: None };
        let marks = BandMarks { segments: vec![marker(c, "centre-align"), marker(r, "right-align")], icons: vec![] };
        let text = TextSlice { extends: vec![], utf8: text.into(), face_runs: vec![], raw: vec![] };
        marks.places(&text, width, 12.0, &vec![10.0; text.utf8.chars().count()])
    }

    /// The centred zone is centred on the band; a side that would come
    /// within the gap pushes it away as little as needed; with no room for
    /// both gaps it follows the left zone, everything clipped where the
    /// right zone starts; the right zone is flush with the padding.
    #[test]
    fn the_centre_zone_centres_yields_and_flows() {
        // "ab|NAME|xy": left 20, centre 40, right 20 in a 400 px band.
        let places = zoned("ab NAME xy", 2, 7, 400.0);
        assert_eq!(places[3], Some((180.0, 10.0)), "centred: 200 - 20");
        assert_eq!(places[9].map(|(x, w)| x + w), Some(388.0), "flush right inside the padding");
        assert_eq!(places[2], Some((32.0, 148.0)), "the centre marker spans the room before the name");
        // A long left side pushes the centre right, the gap kept.
        let places = zoned(&format!("{} NAME xy", "l".repeat(16)), 16, 21, 400.0);
        assert_eq!(places[17].map(|(x, _)| x), Some(12.0 + 160.0 + STATUS_BAR_ZONE_GAP));
        // A long right side pushes it left.
        let places = zoned(&format!("ab NAME {}", "r".repeat(16)), 2, 7, 400.0);
        let right_x = places[8].unwrap().0;
        assert_eq!(places[6].map(|(x, w)| x + w), Some(right_x - STATUS_BAR_ZONE_GAP));
        // Both: the name follows the left side, the right side whole.
        let places = zoned(&format!("{} NAME {}", "l".repeat(15), "r".repeat(15)), 15, 20, 400.0);
        assert_eq!(places[16], Some((12.0 + 150.0, 10.0)), "flows after the left zone");
        assert!(places[21..].iter().all(Option::is_some));
        // Too narrow for all: left and centre clipped where the right starts, a prefix each.
        let places = zoned(&format!("{} NAME xy", "l".repeat(30)), 30, 35, 200.0);
        let right_x = places[36].unwrap().0;
        let shown: Vec<usize> = (0..35).filter(|&i| places[i].is_some() && i != 30).collect();
        assert!(shown.iter().all(|&i| places[i].is_some_and(|(x, w)| x + w <= right_x + 0.01)));
        assert!(places[31..35].iter().all(Option::is_none), "the name after a clipped left zone does not show");
        assert!(shown.windows(2).all(|w| w[1] == w[0] + 1), "a prefix");
    }

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
