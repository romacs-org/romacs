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

/// The embedder's window system, asked synchronously.
pub trait DisplayHost {
    /// The `window-system' symbol's name for frames on this display.
    fn window_system(&self) -> &str;
    /// The display's name, as GNU's frame `display' parameter.
    fn display_name(&self) -> String;
    /// Installed faces matching QUERY, in the platform's order.
    fn list_fonts(&mut self, query: &FontQuery) -> Vec<FontEntity>;
    /// Installed family names.
    fn font_families(&mut self) -> Vec<String>;
    /// ENTITY opened at PIXEL_SIZE, if it opens.
    fn open_font(&mut self, entity: &FontEntity, pixel_size: i64) -> Option<OpenedFont>;
    /// The font and glyph code the frame draws CH with in a face of FAMILY
    /// at PIXEL_SIZE, as painting resolves it; None when no font has CH.
    fn char_font(&mut self, family: &str, pixel_size: f32, ch: char) -> Option<(OpenedFont, u32)>;
}

/// The attached display: closed until the session opens it.
pub(crate) struct Display {
    pub host: Box<dyn DisplayHost>,
    pub open: bool,
    /// The frame font's name for the default face's family and pixel size
    /// it was last asked for: `frame-parameters' names it on every call.
    pub frame_font: Option<((String, u32), String)>,
}

impl Vm {
    /// Attach (Some) or detach (None) the embedder's window system. It
    /// stays closed, the frame an initial-terminal frame, until
    /// [`Vm::open_display`].
    pub fn set_display_host(&mut self, host: Option<Box<dyn DisplayHost>>) {
        self.display = host.map(|host| Display { host, open: false, frame_font: None });
    }
    /// Put the frame on the attached display, as GNU's window-system
    /// initialization does: from here `framep' and `window-system' answer
    /// the display's backend. False when no display is attached.
    pub fn open_display(&mut self) -> bool {
        self.display.as_mut().map(|display| display.open = true).is_some()
    }
    /// The open display's host, if the frame is on one.
    pub(crate) fn window_system_host(&mut self) -> Option<&mut (dyn DisplayHost + 'static)> {
        match self.display.as_mut() { Some(display) if display.open => Some(display.host.as_mut()), _ => None }
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
