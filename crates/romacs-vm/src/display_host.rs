//! The window system a GUI embedder provides (DIVERGENCES.md VM2-WIN5):
//! the backend GNU's frames on it report, its display's name, and the
//! fonts it draws with. The VM asks synchronously, as it asks the layout
//! host; no Lisp value crosses. The display opens once the session leaves
//! the batch chain, so a world, captured before that, never holds it.
use crate::Vm;

/// An installed font face: GNU's font-entity, with styles in font.c's
/// numeric scales (Fontconfig's numbers).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontEntity {
    pub foundry: Option<String>,
    pub family: String,
    /// font.c weight_table: 80 regular, 200 bold.
    pub weight: i64,
    /// font.c slant_table: 100 normal, 200 italic, 210 oblique.
    pub slant: i64,
    /// font.c width_table: 100 normal.
    pub width: i64,
    /// FONT_SPACING_*: 0 proportional, 90 dual, 100 mono, 110 charcell.
    pub spacing: Option<i64>,
    pub registry: String,
    pub file: String,
    pub index: u32,
}

/// A face opened at a pixel size: GNU's font-object metrics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedFont {
    pub entity: FontEntity,
    pub pixel_size: i64,
    pub ascent: i64,
    pub descent: i64,
    pub max_width: i64,
    pub space_width: i64,
    pub average_width: i64,
    /// Its OpenType GSUB and GPOS layout, as hbfont.c reports it.
    pub otf: (OtfTable, OtfTable),
}

/// A glyph as text_extents measures it, in whole pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphMetrics {
    pub code: u32,
    pub width: i64,
    pub lbearing: i64,
    pub rbearing: i64,
    pub ascent: i64,
    pub descent: i64,
}

/// One OpenType layout table: each script's tag with its language systems
/// (None the default), each with its feature tags.
pub type OtfTable = Vec<(String, Vec<(Option<String>, Vec<String>)>)>;

/// What `list-fonts' asks for: each given property must match.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontQuery {
    pub family: Option<String>,
    pub foundry: Option<String>,
    pub weight: Option<i64>,
    pub slant: Option<i64>,
    pub width: Option<i64>,
    pub spacing: Option<i64>,
    pub registry: Option<String>,
}

/// A face's font as painting requests it: its family and pixel size, and
/// its weight (CSS, 400 normal), slant and width (OS/2 class, 5 normal).
#[derive(Clone, Debug, PartialEq)]
pub struct FaceFont {
    pub family: String,
    pub pixel_size: f32,
    pub weight: u16,
    pub italic: bool,
    pub width: u16,
}

impl FaceFont {
    /// The request as a hashable key.
    pub fn key(&self) -> (String, u32, u16, bool, u16) {
        (self.family.clone(), self.pixel_size.to_bits(), self.weight, self.italic, self.width)
    }
}

/// `DisplayHost::finish_decode''s error when the wait was quit.
pub const DECODE_QUIT: &str = "quit";

/// The embedder's window system, asked synchronously.
pub trait DisplayHost {
    /// The `window-system' symbol's name for frames on this display.
    fn window_system(&self) -> &str;
    /// The display's name, as GNU's frame `display' parameter.
    fn display_name(&self) -> String;
    /// Installed faces matching QUERY, in the platform's order.
    fn list_fonts(&mut self, query: &FontQuery) -> Vec<FontEntity>;
    /// FAMILY's fonts (any family's for None) for repeated matching: a host
    /// may keep them, since asking its font database is the slow part of
    /// `display-supports-face-attributes-p'. `list_fonts' stays live.
    fn family_fonts(&mut self, family: Option<&str>) -> std::rc::Rc<Vec<FontEntity>> {
        std::rc::Rc::new(self.list_fonts(&FontQuery { family: family.map(str::to_owned), ..Default::default() }))
    }
    /// Installed family names.
    fn font_families(&mut self) -> Vec<String>;
    /// ENTITY opened at PIXEL_SIZE, if it opens.
    fn open_font(&mut self, entity: &FontEntity, pixel_size: i64) -> Option<OpenedFont>;
    /// The font and glyph code the frame draws CH with in a face asking for
    /// FACE's font, as painting resolves it, its weight, slant and width
    /// included; None when no font has CH.
    fn char_font(&mut self, face: &FaceFont, ch: char) -> Option<(OpenedFont, u32)>;
    /// CH in FONT: its glyph code and metrics; None when FONT lacks it.
    fn glyph_metrics(&mut self, _font: &OpenedFont, _ch: char) -> Option<GlyphMetrics> { None }
    /// Forget every font opened (Fclear_font_cache).
    fn clear_font_cache(&mut self) {}
    /// Adopt STATE: `char_font' answers by its rules from here on.
    fn set_fontsets(&mut self, _state: std::sync::Arc<romacs_layout_input::FontsetState>) {}
    /// The font a fontset's def was realized to, opened.
    fn opened_def_font(&mut self, _font: &romacs_layout_input::OpenedDefFont) -> Option<OpenedFont> { None }
    /// Apply an `alpha' setter of the frame the display knows as FRAME
    /// (pgtkterm.c:4663 pgtk_set_frame_alpha): ALPHA while it is active and
    /// inactive, negative leaving its opacity alone, raised to LIMIT,
    /// `frame-alpha-lower-limit'.
    fn set_frame_alpha(&mut self, _frame: u64, _alpha: [f32; 2], _limit: f32) {}
    /// Show CONFIG, the visual configuration Lisp asked for
    /// (`romacs--effect-set'): cursor motion, transitions, effects.
    fn set_visual_config(&mut self, _config: &romacs_display_protocol::VisualConfig) {}
    /// Decode QUERY's image as GNU's loaders do (`load_img'), keeping its
    /// pixels under the facts' id until `release_image'.  An error is the
    /// loader's message; the image then shows as a placeholder.
    fn decode_image(&mut self, _query: crate::ImageQuery) -> Result<crate::ImageFacts, String> {
        Err("images are not supported on this display".into())
    }
    /// Forget the pixels kept under ID.
    fn release_image(&mut self, _id: u64) {}
    /// Start decoding QUERY for display: the image's facts at once, PENDING
    /// when the host sized it without decoding (its pixels and final facts
    /// follow through `finished_decodes'), else after a decode here.
    fn begin_decode(&mut self, query: crate::ImageQuery) -> Result<crate::ImageFacts, String> {
        self.decode_image(query)
    }
    /// Wait for image ID's pending decode: its final facts, or the loader's
    /// error.  QUIT says when to give up (`C-g'); then the error is
    /// [`DECODE_QUIT`].
    fn finish_decode(&mut self, _id: u64, _quit: &dyn Fn() -> bool) -> Result<crate::ImageFacts, String> {
        Err("no pending decode".into())
    }
    /// The decodes finished since the last call: each image's id and its
    /// final facts or the loader's error.
    fn finished_decodes(&mut self) -> Vec<(u64, Result<crate::ImageFacts, String>)> { vec![] }
    /// The size `image-size` reports at scale 1 for the image file PATH,
    /// from its header alone (`romacs-image-header-size`); None when the
    /// header does not give it.
    fn image_header_size(&mut self, _path: &str) -> Option<(u32, u32)> { None }
}

/// The attached display: closed until the session opens it.
pub(crate) struct Display {
    pub host: Box<dyn DisplayHost>,
    pub open: bool,
    /// The ASCII font's name for each family and pixel size (its bits) a
    /// face realized to: `frame-parameters' and `face-font' name them on
    /// every call.
    pub face_fonts: rustc_hash::FxHashMap<(String, u32, u16, bool, u16), String>,
}

impl Vm {
    /// Attach (Some) or detach (None) the embedder's window system. It
    /// stays closed, the frame an initial-terminal frame, until
    /// [`Vm::open_display`].
    pub fn set_display_host(&mut self, host: Option<Box<dyn DisplayHost>>) {
        self.display = host.map(|host| Display { host, open: false, face_fonts: Default::default() });
    }
    /// Put the frame on the attached display, as GNU's window-system
    /// initialization does: from here `framep' and `window-system' answer
    /// the display's backend. False when no display is attached.
    pub fn open_display(&mut self) -> bool {
        let Some(display) = self.display.as_mut() else { return false };
        display.open = true;
        // What Lisp asked for before the display opened shows on it.
        if *self.visual != romacs_display_protocol::VisualConfig::default() {
            display.host.set_visual_config(&self.visual);
        }
        true
    }
    /// The open display's host, if the frame is on one.
    pub(crate) fn window_system_host(&mut self) -> Option<&mut (dyn DisplayHost + 'static)> {
        match self.display.as_mut() { Some(display) if display.open => Some(display.host.as_mut()), _ => None }
    }
    /// Whether the frame is on an open display: GNU's FRAME_WINDOW_P.
    pub(crate) fn window_system_open(&self) -> bool {
        self.display.as_ref().is_some_and(|display| display.open)
    }
    /// GNU's frame type: the open display's backend symbol, else t.
    pub(crate) fn frame_type(&mut self) -> crate::Value {
        match self.display.as_ref().filter(|display| display.open) {
            Some(display) => {
                let name = display.host.window_system().to_owned();
                self.intern(&name)
            }
            None => crate::Value::TRUE,
        }
    }
}
