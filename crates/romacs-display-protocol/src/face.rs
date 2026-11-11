//! Face (text styling) types.

use crate::geometry::{DeviceScale, LogicalPixels};
use crate::types::{Color, FaceId};
use bitflags::bitflags;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use strum::{EnumString, IntoStaticStr};

/// The realized face table a frame presentation carries, keyed by face id.
///
/// Every consumer joins glyphs to styling through this one map, so it is named
/// once here rather than respelled at each boundary.  The hasher is `Fx`, not
/// the default SipHash: the keys are engine-minted dense ids, never
/// attacker-supplied, and redisplay probes this table once per glyph.
pub type FrameFaceMap = rustc_hash::FxHashMap<FaceId, Face>;

bitflags! {
    /// Face attributes flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FaceAttributes: u32 {
        const BOLD = 1 << 0;
        const ITALIC = 1 << 1;
        const UNDERLINE = 1 << 2;
        const OVERLINE = 1 << 3;
        const STRIKE_THROUGH = 1 << 4;
        const INVERSE = 1 << 5;
        const BOX = 1 << 6;
    }
}

// Serialize as the raw u32 bit pattern: stable, lossless, and a plain
// integer in snapshot JSON (bitflags' own serde feature would emit the
// "BOLD | ITALIC" string form, which is noisier for machine consumers).
impl serde::Serialize for FaceAttributes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.bits())
    }
}

impl<'de> serde::Deserialize<'de> for FaceAttributes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(FaceAttributes::from_bits_retain(u32::deserialize(
            deserializer,
        )?))
    }
}

/// Underline style.
///
/// The numeric values match GNU Emacs `enum face_underline_type` and the
/// `Smulx` terminfo underline style parameter.
#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    IntoPrimitive,
    TryFromPrimitive,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum UnderlineStyle {
    #[default]
    None = 0,
    Line = 1,
    Double = 2,
    Wave = 3,
    Dotted = 4,
    Dashed = 5,
}

/// A non-font face feature whose availability is owned by a graphical
/// renderer.
///
/// This crosses the evaluator/host/renderer boundary as a closed enum so each
/// renderer must handle new feature variants explicitly at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicalFaceAttribute {
    Foreground,
    Background,
    DistantForeground,
    Stipple,
    Underline(UnderlineStyle),
    Overline,
    StrikeThrough,
    Box,
    InverseVideo,
    Extend,
}

impl UnderlineStyle {
    pub fn from_gnu_code(code: u8) -> Option<Self> {
        Self::try_from(code).ok()
    }

    pub fn gnu_code(self) -> u8 {
        self.into()
    }
}

/// Fully realized vertical placement policy for an underline.
///
/// Keeping this as an enum prevents GNU's descent-line request from being
/// collapsed into the font's ordinary baseline-relative metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UnderlinePosition {
    /// Offset from the glyph baseline, supplied by the realized font.
    FontMetric { offset_from_baseline: i32 },
    /// Place the underline at the bottom of the glyph row, inset upward.
    DescentLine { pixels_above: u32 },
}

impl Default for UnderlinePosition {
    fn default() -> Self {
        Self::FontMetric {
            offset_from_baseline: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnderlineGeometry {
    pub top_y: f32,
    pub thickness: f32,
}

impl UnderlinePosition {
    /// Resolve GNU's underline placement and clipping rules for one glyph row.
    pub fn resolve(
        self,
        row_top: f32,
        row_height: f32,
        baseline: f32,
        requested_thickness: f32,
    ) -> UnderlineGeometry {
        let row_height = row_height.max(1.0);
        let row_bottom = row_top + row_height;
        let mut thickness = requested_thickness.max(1.0);
        let offset = match self {
            Self::FontMetric {
                offset_from_baseline,
            } => offset_from_baseline.max(1) as f32,
            Self::DescentLine { pixels_above } => {
                row_height - thickness - (baseline - row_top) - pixels_above as f32
            }
        };
        let mut top_y = baseline + offset;

        // GNU xterm.c keeps decorations inside the current glyph string.
        if row_bottom <= top_y {
            top_y = (row_bottom - 1.0).max(row_top);
        }
        if row_bottom < top_y + thickness {
            thickness = (row_bottom - top_y).max(0.0);
        }

        UnderlineGeometry { top_y, thickness }
    }
}

/// Box type for face
#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    IntoPrimitive,
    TryFromPrimitive,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum BoxType {
    #[default]
    None = 0,
    Line = 1,
    Raised3D = 2,
    Sunken3D = 3,
}

/// Compact participation and vertical-side ownership for one glyph box run.
///
/// GNU stores these as `left_box_line_p` and `right_box_line_p` on every
/// glyph.  Top and bottom are not optional: a boxed face always paints those
/// horizontal edges, including across an `:extend` end-of-line stretch.  This
/// `Unboxed` is distinct from boxed `Neither`: an interior glyph owns no
/// terminal side but still participates in the run. Keeping both facts in one
/// byte preserves the hot `Glyph` layout while preventing bidi from guessing
/// membership from face identity or edge bits.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
pub enum BoxVerticalEdges {
    Unboxed,
    Neither,
    Left,
    Right,
    #[default]
    Both,
}

/// Whether a glyph participates in a boxed face run.
///
/// This is deliberately independent from [`BoxVerticalEdges`]: an interior
/// glyph in a continuing run owns neither vertical side, while an unboxed
/// glyph also owns neither.  Bidi reordering needs this typed fact to rebuild
/// terminals from visual boxed/unboxed adjacency without guessing from edge
/// bits or face identity.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
pub enum BoxRunMembership {
    #[default]
    Unboxed,
    Boxed,
}

impl BoxRunMembership {
    pub const fn from_boxed(boxed: bool) -> Self {
        if boxed { Self::Boxed } else { Self::Unboxed }
    }

    pub const fn is_boxed(self) -> bool {
        matches!(self, Self::Boxed)
    }
}

impl BoxVerticalEdges {
    #[cfg(test)]
    /// Stable compact representation for retained-row identity.
    pub(crate) const fn hash_code(self) -> u8 {
        match self {
            Self::Unboxed => 0,
            Self::Neither => 4,
            Self::Left => 5,
            Self::Right => 6,
            Self::Both => 7,
        }
    }

    pub const fn membership(self) -> BoxRunMembership {
        BoxRunMembership::from_boxed(!matches!(self, Self::Unboxed))
    }

    pub const fn with_membership(self, membership: BoxRunMembership) -> Self {
        match membership {
            BoxRunMembership::Unboxed => Self::Unboxed,
            BoxRunMembership::Boxed => match self {
                Self::Unboxed => Self::Neither,
                edges => edges,
            },
        }
    }

    pub const fn from_ownership(left: bool, right: bool) -> Self {
        match (left, right) {
            (false, false) => Self::Neither,
            (true, false) => Self::Left,
            (false, true) => Self::Right,
            (true, true) => Self::Both,
        }
    }

    pub const fn owns_left(self) -> bool {
        matches!(self, Self::Left | Self::Both)
    }

    pub const fn owns_right(self) -> bool {
        matches!(self, Self::Right | Self::Both)
    }

    /// Convert logical source-side ownership to physical ownership when a
    /// whole display row is reversed. GNU's `append_stretch_glyph` performs
    /// the same start/end swap for R2L rows.
    pub const fn reversed(self) -> Self {
        if matches!(self, Self::Unboxed) {
            Self::Unboxed
        } else {
            Self::from_ownership(self.owns_right(), self.owns_left())
        }
    }

    /// Merge adjacent glyph contributions into one run: only the first
    /// contribution can own the run's left side and only the last can own its
    /// right side.
    pub const fn merged_with(self, last: Self) -> Self {
        if matches!(self, Self::Unboxed) && matches!(last, Self::Unboxed) {
            Self::Unboxed
        } else {
            Self::from_ownership(self.owns_left(), last.owns_right())
        }
    }
}

impl BoxType {
    pub fn from_gnu_code(code: u8) -> Option<Self> {
        Self::try_from(code).ok()
    }

    pub fn gnu_code(self) -> u8 {
        self.into()
    }
}

/// GNU Emacs scalar `:box :line-width` semantics.
///
/// The magnitude is the painted border thickness in device pixels. A positive
/// value expands the glyph row vertically; a negative value paints the top and
/// bottom edges inside the existing row. GNU uses the absolute magnitude for
/// the left and right edges in both cases
/// (`struct face::box_{vertical,horizontal}_line_width`).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct BoxLineWidth(i32);

/// Logical geometry derived from a GNU `:box :line-width` for one surface.
///
/// GNU stores box widths as device pixels, while Romacs layout and WGPU
/// vertices use logical pixels. Keeping the conversion behind this type makes
/// it impossible for those consumers to use the raw GNU magnitude without
/// naming the device scale that gives it meaning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxLineGeometry {
    paint_thickness: LogicalPixels,
    row_expansion_per_edge: LogicalPixels,
}

impl BoxLineGeometry {
    #[must_use]
    pub const fn paint_thickness(self) -> LogicalPixels {
        self.paint_thickness
    }

    #[must_use]
    pub const fn row_expansion_per_edge(self) -> LogicalPixels {
        self.row_expansion_per_edge
    }
}

impl BoxLineWidth {
    pub const fn from_gnu(value: i32) -> Self {
        Self(value)
    }

    pub const fn gnu_value(self) -> i32 {
        self.0
    }

    pub const fn is_visible(self) -> bool {
        self.0 != 0
    }

    pub const fn expands_row_height(self) -> bool {
        self.0 > 0
    }

    /// Resolve this GNU device-pixel width into logical layout/render geometry.
    #[must_use]
    pub fn logical_geometry(self, device_scale: DeviceScale) -> BoxLineGeometry {
        let paint_thickness = self.0.unsigned_abs() as f32 / device_scale.get();
        let row_expansion_per_edge = if self.expands_row_height() {
            paint_thickness
        } else {
            0.0
        };
        BoxLineGeometry {
            paint_thickness: LogicalPixels::new(paint_thickness)
                .expect("a finite box width divided by a valid scale stays finite"),
            row_expansion_per_edge: LogicalPixels::new(row_expansion_per_edge)
                .expect("box row expansion is finite"),
        }
    }
}

impl From<i32> for BoxLineWidth {
    fn from(value: i32) -> Self {
        Self::from_gnu(value)
    }
}

/// Fancy box border style — a Romacs extension to GNU's box faces.
///
/// The numeric codes are the wire values used by the C-side
/// `fill_face_data()` (`FaceDataFFI::box_border_style`) and by the
/// GPU border shader's style id.
#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    IntoPrimitive,
    TryFromPrimitive,
    serde::Serialize,
    serde::Deserialize,
)]
#[non_exhaustive]
pub enum BoxBorderStyle {
    #[default]
    Solid = 0,
    Rainbow = 1,
    AnimatedRainbow = 2,
    Gradient = 3,
    Glow = 4,
    Neon = 5,
    Dashed = 6,
    Comet = 7,
    Iridescent = 8,
    Fire = 9,
    Heartbeat = 10,
}

impl BoxBorderStyle {
    pub fn from_gnu_code(code: u32) -> Option<Self> {
        Self::try_from(code).ok()
    }

    pub fn gnu_code(self) -> u32 {
        self.into()
    }

    /// True for any animated/fancy style (everything except plain solid).
    pub fn is_fancy(self) -> bool {
        self != Self::Solid
    }
}

/// Basic face IDs — fixed cache slots matching GNU's `enum face_id`.
///
/// Realized at frame creation via `realize_basic_faces()` in GNU
/// (`src/xfaces.c`).  Romacs mirrors this: basic faces always occupy
/// IDs 0–19 in every frame's face cache; dynamic faces start at
/// [`BasicFaceId::SENTINEL`].
#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    EnumString,
    IntoPrimitive,
    IntoStaticStr,
    TryFromPrimitive,
)]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum BasicFaceId {
    Default = 0,
    ModeLineActive = 1,
    ModeLineInactive = 2,
    ToolBar = 3,
    Fringe = 4,
    HeaderLineActive = 5,
    HeaderLineInactive = 6,
    ScrollBar = 7,
    Border = 8,
    Cursor = 9,
    Mouse = 10,
    Menu = 11,
    VerticalBorder = 12,
    WindowDivider = 13,
    WindowDividerFirstPixel = 14,
    WindowDividerLastPixel = 15,
    InternalBorder = 16,
    ChildFrameBorder = 17,
    TabBar = 18,
    TabLine = 19,
}

impl BasicFaceId {
    /// One past the last basic face.  Dynamic face IDs start here.
    pub(crate) const SENTINEL: u32 = 20;

    #[cfg(test)]
    /// Look up a basic face by its canonical name.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        name.parse().ok()
    }

    /// Look up a basic face by GNU's fixed `enum face_id` value.
    pub fn from_gnu_code(code: u32) -> Option<Self> {
        Self::try_from(code).ok()
    }

    /// Return the canonical basic face name.
    pub fn name(self) -> &'static str {
        self.into()
    }

    /// Return GNU's fixed `enum face_id` value.
    pub fn gnu_code(self) -> u32 {
        self.into()
    }
}

/// Basic faces occupy their fixed cache slot in every frame's face table.
impl From<BasicFaceId> for FaceId {
    fn from(basic: BasicFaceId) -> Self {
        FaceId::new(basic.gnu_code())
    }
}

impl FaceId {
    /// The default face's slot in every frame's face table.
    #[must_use]
    pub fn of_default() -> Self {
        BasicFaceId::Default.into()
    }
}

/// A face defines text styling (colors, font, decorations)
#[repr(C)]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Face {
    /// Face ID
    pub id: FaceId,

    /// Foreground color
    pub foreground: Color,

    /// Background color
    pub background: Color,

    /// What a TERMINAL frame writes for [`Self::foreground`]: the index
    /// `tty-color-desc` returned, which is the whole of GNU's
    /// `face->foreground` on a tty frame (`map_tty_color`,
    /// src/xfaces.c:6620-6694) and the only thing `turn_on_face` emits
    /// (src/term.c:2093-2117).
    ///
    /// `None` on a GUI frame, and on a tty for a colour `tty-color-desc` could
    /// not resolve -- GNU's `FACE_TTY_DEFAULT_COLOR`, which
    /// `face_tty_specified_color` (src/dispextern.h:1933-1936) rejects, so the
    /// writer emits no colour rather than one it invented.
    #[serde(default)]
    pub terminal_foreground: Option<crate::terminal_color::TerminalColor>,

    /// What a TERMINAL frame writes for [`Self::background`]; see
    /// [`Self::terminal_foreground`].
    #[serde(default)]
    pub terminal_background: Option<crate::terminal_color::TerminalColor>,

    /// Use the terminal's default foreground instead of `foreground`.
    pub use_default_foreground: bool,

    /// Use the terminal's default background instead of `background`.
    pub use_default_background: bool,

    /// Underline color (if different from foreground)
    pub underline_color: Option<Color>,

    /// What a TERMINAL frame writes for the underline colour: GNU's
    /// `face->underline_color` (src/dispextern.h:1811), realized through the
    /// same `map_tty_color` as the foreground and the background
    /// (src/xfaces.c:6748 and :6777) and emitted by `turn_on_face` through
    /// `TF_set_underline_color` (src/term.c:2119-2126).
    ///
    /// It is NOT [`Self::underline_color`] quantized.  That field is the GUI's,
    /// and it defaults to the face's foreground so a `:underline t` draws in
    /// the text colour; GNU's terminal slot is 0 in exactly that case and
    /// `turn_on_face` then emits nothing.  Keeping them apart is what stops a
    /// plain underline from acquiring a colour on a terminal.
    ///
    /// `None` on a GUI frame, for an underline with no `:color` of its own, and
    /// for `(:color foreground-color)` -- GNU zeroes the slot for all three
    /// (src/xfaces.c:6741, :6756, :6772-6773).
    #[serde(default)]
    pub terminal_underline_color: Option<crate::terminal_color::TerminalColor>,

    /// Overline color
    pub overline_color: Option<Color>,

    /// Strike-through color
    pub strike_through_color: Option<Color>,

    /// Box color
    pub box_color: Option<Color>,

    /// Font family name
    pub font_family: String,

    /// Font size in points (1/72 inch)
    pub font_size: f32,

    /// Font weight (400 = normal, 700 = bold)
    pub font_weight: u16,

    /// Attribute flags
    pub attributes: FaceAttributes,

    /// Underline style
    pub underline_style: UnderlineStyle,

    /// Box type
    pub box_type: BoxType,

    /// GNU scalar box line width, including inside/outside sign semantics.
    pub box_line_width: BoxLineWidth,

    /// The left and right edges' width where `:line-width (VWIDTH . HWIDTH)'
    /// gives them their own (`face->box_vertical_line_width'); else
    /// `box_line_width''s magnitude.
    pub box_vertical_line_width: Option<BoxLineWidth>,

    /// Box corner radius (0 = sharp corners)
    pub box_corner_radius: i32,

    /// Fancy border style; see [`BoxBorderStyle`].
    pub box_border_style: BoxBorderStyle,

    /// Animation speed multiplier for fancy border effects (default 1.0)
    pub box_border_speed: f32,

    /// Secondary box color (for gradient, neon, etc.)
    pub box_color2: Option<Color>,

    /// Absolute path to the resolved font file (from Fontconfig), if available.
    /// Used to pre-load the exact font file into cosmic-text's fontdb.
    pub font_file_path: Option<String>,

    /// Font metrics from Emacs's realized font
    /// Font ascent (FONT_BASE) in pixels
    pub font_ascent: i32,
    /// Font descent (FONT_DESCENT) in pixels
    pub font_descent: i32,
    /// Underline position below baseline (font->underline_position).
    ///
    /// This legacy scalar remains in the stable `#[repr(C)]` prefix. New code
    /// uses [`Self::underline_placement`] so descent-line placement cannot be
    /// confused with a font metric.
    pub underline_position: i32,
    /// Underline thickness (font->underline_thickness)
    pub underline_thickness: i32,

    /// Optional GPU-rendered gradient background.
    /// If present, this overrides the solid `background` color during rendering.
    /// GPU fragment shader evaluates the gradient per-pixel with no CPU overhead.
    pub background_gradient: Option<Box<crate::gradient::Gradient>>,

    /// Lisp face name this realized face came from (e.g.
    /// "font-lock-keyword-face"), when known. `None` for anonymous faces
    /// realized from raw attribute plists. Basic faces (id 0-19) fall back
    /// to their canonical [`BasicFaceId`] name at snapshot time.
    ///
    /// NOTE: new fields append AFTER this one so the `#[repr(C)]` prefix
    /// layout stays stable for existing readers.
    pub lisp_name: Option<String>,

    /// The face's primary resolved font, referencing the frame state's
    /// resolved font table (`FrameDisplayState::fonts`). When present, the
    /// renderer must rasterize with exactly this font instead of re-running
    /// semantic selection from `font_family`/`font_weight`/attributes.
    pub default_resolved_font_id: Option<crate::font::ResolvedFontId>,

    /// Stipple bitmap painted as the glyph background (GNU `face->stipple`).
    /// When present the renderer tiles this XBM pattern over the glyph's
    /// background rect, painting 1-bits in the face foreground over the solid
    /// background (`highlight-indent-guides`/`indent-bars` rely on this).
    /// Boxed to keep `Face` small, matching `background_gradient`.
    #[serde(default)]
    pub stipple: Option<Box<crate::frame_glyphs::StipplePattern>>,

    /// Typed GNU-compatible vertical placement policy.
    #[serde(default)]
    pub underline_placement: UnderlinePosition,
}

impl Default for Face {
    fn default() -> Self {
        Self {
            id: FaceId::new(0),
            foreground: Color::WHITE,
            background: Color::BLACK,
            terminal_foreground: None,
            terminal_background: None,
            use_default_foreground: false,
            use_default_background: false,
            underline_color: None,
            terminal_underline_color: None,
            overline_color: None,
            strike_through_color: None,
            box_color: None,
            font_family: "monospace".to_string(),
            font_size: 10.0,
            font_weight: 400,
            attributes: FaceAttributes::empty(),
            underline_style: UnderlineStyle::None,
            box_type: BoxType::None,
            box_line_width: BoxLineWidth::default(),
            box_vertical_line_width: None,
            box_corner_radius: 0,
            box_border_style: BoxBorderStyle::Solid,
            box_border_speed: 1.0,
            box_color2: None,
            font_file_path: None,
            font_ascent: 0,
            font_descent: 0,
            underline_position: 1,
            underline_thickness: 1,
            background_gradient: None,
            lisp_name: None,
            default_resolved_font_id: None,
            stipple: None,
            underline_placement: UnderlinePosition::default(),
        }
    }
}

impl Face {
    /// Create a new face with default values
    pub fn new(id: FaceId) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }

    #[cfg(test)]
    /// Check if face is bold
    pub(crate) fn is_bold(&self) -> bool {
        self.attributes.contains(FaceAttributes::BOLD) || self.font_weight >= 700
    }

    #[cfg(test)]
    /// Check if face is italic
    pub(crate) fn is_italic(&self) -> bool {
        self.attributes.contains(FaceAttributes::ITALIC)
    }

    /// Check if face has underline
    pub fn has_underline(&self) -> bool {
        self.underline_style != UnderlineStyle::None
    }

    #[cfg(test)]
    /// Get the underline color (foreground if not explicitly set)
    pub(crate) fn get_underline_color(&self) -> Color {
        self.underline_color.unwrap_or(self.foreground)
    }
}

/// Face cache for efficient lookup
#[derive(Debug, Default)]
pub struct FaceCache {
    faces: Vec<Face>,
}

impl FaceCache {
    pub fn new() -> Self {
        Self { faces: Vec::new() }
    }

    /// Get face by ID
    pub fn get(&self, id: FaceId) -> Option<&Face> {
        self.faces.iter().find(|f| f.id == id)
    }

    #[cfg(test)]
    /// Get or create a face by ID
    pub(crate) fn get_or_create(&mut self, id: FaceId) -> &Face {
        // Check if exists
        if self.get(id).is_some() {
            return self.get(id).unwrap();
        }
        // Create new
        let face = Face::new(id);
        self.faces.push(face);
        self.faces.last().unwrap()
    }

    /// Add or update a face, returns the face ID
    pub fn insert(&mut self, face: Face) -> FaceId {
        let id = face.id;
        if let Some(existing) = self.faces.iter_mut().find(|f| f.id == face.id) {
            *existing = face;
        } else {
            self.faces.push(face);
        }
        id
    }

    #[cfg(test)]
    /// Get default face (ID 0)
    pub(crate) fn default_face(&self) -> Option<&Face> {
        self.get(FaceId::new(0))
    }
}

#[cfg(test)]
mod tests;
