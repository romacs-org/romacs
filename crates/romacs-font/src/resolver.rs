//! Plain-data face and fontset → exact resolved fonts.

#[cfg(test)]
#[path = "resolver/tests/selection_order_test.rs"]
mod selection_order_tests;
#[cfg(test)]
#[path = "resolver/tests/alternative_state_test.rs"]
mod alternative_state_tests;

use crate::fontset::{FontsetEntry, FontsetPolicy};
use crate::instance::{FontInstanceInterner, FontInstanceProperties};
use crate::style::{FontRequest, RequestKey, style_score};
use romacs_layout_input::{FontAlternativeState, FontSelectionAttribute, FontSelectionState};

#[cfg(test)]
const DEFAULT_SELECTION_ORDER: [FontSelectionAttribute; 4] = [
    FontSelectionAttribute::Width,
    FontSelectionAttribute::Height,
    FontSelectionAttribute::Weight,
    FontSelectionAttribute::Slant,
];
use romacs_display_protocol::font::{
    FontFileAsset, FontOutlineAsset, FontReplay, ResolvedCharGlyph, ResolvedFont,
    ResolvedFontAdvance, ResolvedFontIdentity, ResolvedGlyphId,
};
use rustc_hash::FxHashMap as HashMap;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FontResolveError {
    #[error("the platform font catalog is unavailable")]
    CatalogUnavailable,
    #[error("no installed scalable font family matches {family:?}")]
    FamilyNotFound { family: String },
    #[error("failed to open {file}#{selector}: {reason}")]
    OpenFailed {
        file: String,
        selector: u32,
        reason: String,
    },
    #[error("no installed font has a glyph for U+{code:04X}")]
    NoCoveringFont { code: u32 },
}

/// Which rule chose a character's font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharResolutionSource {
    /// The `n`th entry the fontset lists for the character.
    Fontset(usize),
    /// No fontset entry applied; the face's own font covers it.
    Primary,
    /// The platform-wide fallback: any installed font with the glyph.
    Fallback,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CharResolution {
    pub font: ResolvedFont,
    pub glyph: ResolvedCharGlyph,
    pub source: CharResolutionSource,
}

#[cfg(target_os = "linux")]
pub(crate) use crate::fontconfig_catalog::{self as catalog, ListedFace};

#[cfg(not(target_os = "linux"))]
pub(crate) mod catalog {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct ListedFace {
        pub family: String,
        pub file: String,
        pub selector: u32,
        pub postscript_name: Option<String>,
        pub weight: u16,
        pub slant: romacs_display_protocol::font::FontSlantKind,
        pub width: u16,
        pub spacing: Option<i32>,
        pub foundry: Option<String>,
        pub fc_style: (i32, i32, i32),
    }
    impl ListedFace {
        pub fn is_monospace(&self) -> bool {
            false
        }
    }
    pub(crate) fn handle() -> Option<()> {
        None
    }
    pub(crate) fn list_faces(_: Option<&str>, _: Option<char>) -> Vec<ListedFace> {
        Vec::new()
    }
    pub(crate) fn match_family(_: &str) -> Option<String> {
        None
    }
    pub(crate) fn sort_faces(_: &str, _: char) -> Vec<ListedFace> {
        Vec::new()
    }
    pub(crate) fn list_families() -> Vec<String> {
        Vec::new()
    }
}
#[cfg(not(target_os = "linux"))]
use catalog::ListedFace;

/// Generic family names Fontconfig resolves through alias rules rather than
/// listing (GNU `ftfont_resolve_generic_family`).
fn is_generic_family(family: &str) -> bool {
    [
        "monospace",
        "mono",
        "sans-serif",
        "sans serif",
        "sans",
        "serif",
    ]
    .iter()
    .any(|generic| generic.eq_ignore_ascii_case(family.trim()))
}

/// One opened exact face at one pixel size: glyph lookup and advance from
/// the same file face the metrics came from.
enum OpenedFace {
    /// Scalable outlines: FreeType's hinted advance, as GNU's Cairo backend.
    #[cfg(target_os = "linux")]
    FreeType(freetype::Face),
    /// Faces FreeType cannot size freely (colour bitmap strikes such as
    /// Noto Color Emoji's CBDT): design-unit advances scaled to the size.
    Sfnt {
        data: std::sync::Arc<Vec<u8>>,
        index: u32,
        pixel_size: f32,
    },
}

impl OpenedFace {
    fn glyph(&self, ch: char) -> Option<(ResolvedGlyphId, f32)> {
        match self {
            #[cfg(target_os = "linux")]
            Self::FreeType(face) => {
                let glyph = face
                    .get_char_index(ch as usize)
                    .filter(|&glyph| glyph != 0)?;
                face.load_glyph(glyph, freetype::face::LoadFlag::DEFAULT)
                    .ok()?;
                // GNU's Cairo/FreeType backend rounds the hinted 26.6 advance
                // to whole pixels (`ftcrfont_glyph_extents`, lround).
                let advance = (face.glyph().metrics().horiAdvance + 32) >> 6;
                Some((ResolvedGlyphId::new(glyph), advance as f32))
            }
            Self::Sfnt {
                data,
                index,
                pixel_size,
            } => {
                let face = ttf_parser::Face::parse(data, *index).ok()?;
                let glyph = face.glyph_index(ch).filter(|glyph| glyph.0 != 0)?;
                let units = f32::from(face.glyph_hor_advance(glyph)?);
                let advance = (units * pixel_size / f32::from(face.units_per_em())).round();
                Some((ResolvedGlyphId::from(glyph.0), advance))
            }
        }
    }
}

/// Vertical metrics and space advance from the sfnt tables, for faces
/// FreeType cannot size at an arbitrary pixel size.
fn sfnt_metrics(data: &[u8], index: u32, pixel_size: f32) -> Option<crate::probe::FontPxMetrics> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    let scale = pixel_size / f32::from(face.units_per_em());
    let px = |units: f32| (units * scale).round() as i32;
    let ascent = px(f32::from(face.ascender())).max(0);
    let descent = px(-f32::from(face.descender())).max(0);
    let space = face
        .glyph_index(' ')
        .and_then(|glyph| face.glyph_hor_advance(glyph))
        .map_or(0, |units| px(f32::from(units)));
    Some(crate::probe::FontPxMetrics {
        pixel_size: pixel_size.round().max(1.0) as u32,
        height: (ascent + descent).max(1),
        ascent,
        descent,
        max_width: space,
        space_width: space,
        average_width: space,
    })
}

/// Resolves faces and characters to exact fonts. Holds FreeType faces, so it
/// is single-threaded: one resolver per layout thread.
pub struct FontResolver {
    #[cfg(target_os = "linux")]
    library: freetype::Library,
    interner: FontInstanceInterner,
    selection_order: FontSelectionState,
    alternatives: FontAlternativeState,
    primary: HashMap<(RequestKey, u64), Result<ResolvedFont, FontResolveError>>,
    chars: HashMap<(RequestKey, u64, char), Result<CharResolution, FontResolveError>>,
    opened: HashMap<(String, u32, u32), Option<OpenedFace>>,
    files: HashMap<String, Option<std::sync::Arc<Vec<u8>>>>,
}

impl std::fmt::Debug for FontResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FontResolver")
            .field("primary_cached", &self.primary.len())
            .field("chars_cached", &self.chars.len())
            .finish()
    }
}

impl FontResolver {
    pub fn new() -> Result<Self, FontResolveError> {
        catalog::handle().ok_or(FontResolveError::CatalogUnavailable)?;
        Ok(Self {
            #[cfg(target_os = "linux")]
            library: freetype::Library::init().map_err(|_| FontResolveError::CatalogUnavailable)?,
            interner: FontInstanceInterner::default(),
            selection_order: FontSelectionState::default(),
            alternatives: FontAlternativeState::default(),
            primary: HashMap::default(),
            chars: HashMap::default(),
            opened: HashMap::default(),
            files: HashMap::default(),
        })
    }

    /// The files this resolver's answers so far came from: the font files
    /// it read and Fontconfig's configuration and font directories, sorted.
    pub fn font_inputs(&self) -> Vec<String> {
        let mut inputs: Vec<String> = self.files.keys().chain(self.opened.keys().map(|(file, _, _)| file)).cloned()
            .chain(catalog::configuration_inputs()).collect();
        inputs.sort();
        inputs.dedup();
        inputs
    }

    /// Current raw order; effective shifts also depend on earlier orders.
    pub fn selection_order(&self) -> [FontSelectionAttribute; 4] {
        self.selection_order.order()
    }

    pub fn selection_state(&self) -> FontSelectionState { self.selection_order }

    /// Adopt the producer's complete history, even across skipped layouts.
    pub fn set_selection_state(&mut self, state: FontSelectionState) {
        if self.selection_order != state {
            self.selection_order = state;
            self.primary.clear();
            self.chars.clear();
        }
    }

    pub fn alternative_state(&self) -> &FontAlternativeState { &self.alternatives }

    /// Adopt the producer's immutable family alist and setter history.
    pub fn set_alternative_state(&mut self, state: FontAlternativeState) {
        if self.alternatives != state {
            self.alternatives = state;
            self.primary.clear();
            self.chars.clear();
        }
    }

    /// Change style priorities and discard answers selected under the old order.
    /// Opened font files and interned exact instances remain reusable.
    pub fn set_selection_order(&mut self, order: [FontSelectionAttribute; 4]) {
        let mut state = self.selection_order;
        state.set_order(order);
        self.set_selection_state(state);
    }

    /// The face's own font: the best-scoring scalable face of the request's
    /// family, then of each `face-font-family-alternatives` entry.
    pub fn resolve_primary(
        &mut self,
        request: &FontRequest,
        policy: &FontsetPolicy,
    ) -> Result<ResolvedFont, FontResolveError> {
        let key = (request.cache_key(), policy.generation());
        if let Some(cached) = self.primary.get(&key) {
            return cached.clone();
        }
        let resolved = self.resolve_primary_uncached(request, policy);
        self.primary.insert(key, resolved.clone());
        resolved
    }

    fn resolve_primary_uncached(
        &mut self,
        request: &FontRequest,
        policy: &FontsetPolicy,
    ) -> Result<ResolvedFont, FontResolveError> {
        for family in policy.families_for(&request.family) {
            let faces = list_family(&family, None);
            if let Some(best) = best_face(&faces, request, self.selection_order, |_| 0) {
                return self.instance(best, request.pixel_size);
            }
        }
        // GNU realizes a face whose family (and its alternatives) is not
        // installed with a substitute rather than failing (measured: "DejaVu
        // Sans" and "No Such Family F0a" both draw in Liberation Sans here).
        // Fontconfig's match for the family pattern gives that substitute on
        // this host; GNU's own route is `font_find_for_lface` dropping the
        // family and scoring every font, which can differ elsewhere.
        if let Some(substitute) = catalog::match_family(&request.family) {
            let faces = list_family(&substitute, None);
            if let Some(best) = best_face(&faces, request, self.selection_order, |_| 0) {
                return self.instance(best, request.pixel_size);
            }
        }
        Err(FontResolveError::FamilyNotFound {
            family: request.family.clone(),
        })
    }

    /// The font for one non-ASCII character drawn in REQUEST's face.
    ///
    /// Order: the fontset's entries for CH (range-specific, then fallback);
    /// then the face's own font when it has the glyph; then any installed
    /// font with the glyph. An explicit nil entry stops the search.
    pub fn resolve_char(
        &mut self,
        request: &FontRequest,
        ch: char,
        policy: &FontsetPolicy,
    ) -> Result<CharResolution, FontResolveError> {
        let key = (request.cache_key(), policy.generation(), ch);
        if let Some(cached) = self.chars.get(&key) {
            return cached.clone();
        }
        let resolved = self.resolve_char_uncached(request, ch, policy);
        self.chars.insert(key, resolved.clone());
        resolved
    }

    fn resolve_char_uncached(
        &mut self,
        request: &FontRequest,
        ch: char,
        policy: &FontsetPolicy,
    ) -> Result<CharResolution, FontResolveError> {
        let code = u32::from(ch);
        let entries: Vec<FontsetEntry> = policy.entries_for(ch).cloned().collect();
        for (index, entry) in entries.iter().enumerate() {
            let spec = match entry {
                // GNU `(set-fontset-font FONTSET TARGET nil)` leaves the
                // range without a font of its own; display still finds one
                // through the fallback, the face font and the platform
                // (measured: Ж stays on Iosevka). Not a stop.
                FontsetEntry::ExplicitNone => continue,
                FontsetEntry::Font(spec) => spec,
            };
            let styled = FontRequest {
                weight: spec.weight.unwrap_or(request.weight),
                slant: spec.slant.unwrap_or(request.slant),
                ..request.clone()
            };
            let families = match &spec.family {
                Some(family) => vec![family.clone()],
                None => policy.families_for(&request.family),
            };
            for family in families {
                let faces = list_family(&family, Some(ch));
                if let Some(found) = self.first_with_glyph(&faces, &styled, ch, |_| 0)? {
                    return Ok(CharResolution {
                        source: CharResolutionSource::Fontset(index),
                        ..found
                    });
                }
            }
        }

        let primary = self.resolve_primary(request, policy)?;
        if let Some((glyph_id, advance_px)) = self.glyph_in(&primary, ch) {
            return Ok(CharResolution {
                glyph: ResolvedCharGlyph {
                    resolved_font_id: primary.id,
                    glyph_id,
                    advance_px,
                },
                font: primary,
                source: CharResolutionSource::Primary,
            });
        }

        // Platform fallback, in Fontconfig's preference order for the face's
        // family with CH required. Measured against GNU 32.0.50 PGTK: 😀
        // under a monospace face lands on Noto Color Emoji in both; plain
        // enumeration order picked an icon font instead.
        for face in catalog::sort_faces(&primary.family, ch) {
            let font = match self.instance(&face, request.pixel_size) {
                Ok(font) => font,
                Err(error) => {
                    tracing::debug!(target: "font_resolver", %error, "skipping unopenable fallback");
                    continue;
                }
            };
            if let Some((glyph_id, advance_px)) = self.glyph_in(&font, ch) {
                return Ok(CharResolution {
                    glyph: ResolvedCharGlyph {
                        resolved_font_id: font.id,
                        glyph_id,
                        advance_px,
                    },
                    font,
                    source: CharResolutionSource::Fallback,
                });
            }
        }
        Err(FontResolveError::NoCoveringFont { code })
    }

    /// Candidates in score order; the first that opens and has CH wins.
    fn first_with_glyph(
        &mut self,
        faces: &[ListedFace],
        request: &FontRequest,
        ch: char,
        compatibility: impl Fn(&ListedFace) -> u32,
    ) -> Result<Option<CharResolution>, FontResolveError> {
        let mut ranked: Vec<((u32, u32), usize)> = faces
            .iter()
            .enumerate()
            .map(|(index, face)| {
                (
                    score(face, request, &compatibility).ordered(self.selection_order),
                    index,
                )
            })
            .collect();
        ranked.sort();
        for (_, index) in ranked {
            let font = match self.instance(&faces[index], request.pixel_size) {
                Ok(font) => font,
                Err(error) => {
                    tracing::debug!(target: "font_resolver", %error, "skipping unopenable candidate");
                    continue;
                }
            };
            if let Some((glyph_id, advance_px)) = self.glyph_in(&font, ch) {
                return Ok(Some(CharResolution {
                    glyph: ResolvedCharGlyph {
                        resolved_font_id: font.id,
                        glyph_id,
                        advance_px,
                    },
                    font,
                    source: CharResolutionSource::Fallback,
                }));
            }
        }
        Ok(None)
    }

    fn glyph_in(&mut self, font: &ResolvedFont, ch: char) -> Option<(ResolvedGlyphId, f32)> {
        let file = font.identity.file_path.clone()?;
        let selector = font.identity.freetype_selector()?;
        self.opened_face(&file, selector, font.pixel_size)?
            .glyph(ch)
    }

    fn file_bytes(&mut self, file: &str) -> Option<std::sync::Arc<Vec<u8>>> {
        self.files
            .entry(file.to_owned())
            .or_insert_with(|| std::fs::read(file).ok().map(std::sync::Arc::new))
            .clone()
    }

    fn opened_face(&mut self, file: &str, selector: u32, pixel_size: f32) -> Option<&OpenedFace> {
        let px = pixel_size.round().max(1.0) as u32;
        let key = (file.to_owned(), selector, px);
        if !self.opened.contains_key(&key) {
            let opened = self.open_face(file, selector, px);
            self.opened.insert(key.clone(), opened);
        }
        self.opened.get(&key)?.as_ref()
    }

    fn open_face(&mut self, file: &str, selector: u32, px: u32) -> Option<OpenedFace> {
        #[cfg(target_os = "linux")]
        if let Ok(face) = self.library.new_face(file, selector as isize)
            && face.is_scalable()
            && face.set_pixel_sizes(0, px).is_ok()
        {
            return Some(OpenedFace::FreeType(face));
        }
        Some(OpenedFace::Sfnt {
            data: self.file_bytes(file)?,
            index: selector & 0xffff,
            pixel_size: px as f32,
        })
    }

    /// Publish one listed face at PIXEL_SIZE as a canonical record.
    fn instance(
        &mut self,
        face: &ListedFace,
        pixel_size: f32,
    ) -> Result<ResolvedFont, FontResolveError> {
        let open_failed = |reason: &str| FontResolveError::OpenFailed {
            file: face.file.clone(),
            selector: face.selector,
            reason: reason.to_owned(),
        };
        let coords = crate::probe::named_instance_variation_coords(&face.file, face.selector);
        let weight = coords
            .iter()
            .find(|coord| coord.tag() == u32::from_be_bytes(*b"wght"))
            .map_or(face.weight, |coord| {
                coord.value().round().clamp(1.0, 1000.0) as u16
            });
        let identity = ResolvedFontIdentity::from_file_with_variations(
            &face.file,
            face.selector,
            face.postscript_name.clone(),
            coords,
        );
        let asset =
            FontFileAsset::from_identity(&identity).ok_or_else(|| open_failed("no file asset"))?;
        let metrics = match crate::probe::probe_font_px_metrics(
            &face.file,
            face.selector,
            pixel_size.round().max(1.0) as u32,
            None,
        ) {
            Some(metrics) => metrics,
            None => self
                .file_bytes(&face.file)
                .and_then(|data| sfnt_metrics(&data, face.selector & 0xffff, pixel_size))
                .ok_or_else(|| open_failed("neither FreeType nor the sfnt tables size the face"))?,
        };
        let replay = FontReplay::Swash {
            asset: FontOutlineAsset::File(asset),
        };
        let glyph_advance = if face.is_monospace() {
            ResolvedFontAdvance::monospace_cells(metrics.max_width as f32)
        } else {
            ResolvedFontAdvance::PerGlyph
        };
        let family = face.family.clone();
        let postscript_name = face.postscript_name.clone();
        let (slant, width) = (face.slant, face.width);
        Ok(self
            .interner
            .intern(identity, replay, pixel_size, || FontInstanceProperties {
                family,
                full_name: None,
                postscript_name,
                weight,
                slant,
                width,
                ascent_px: metrics.ascent.max(0) as f32,
                descent_px: metrics.descent.max(0) as f32,
                space_advance_px: metrics.space_width.max(0) as f32,
                glyph_advance,
            }))
    }
}

/// Faces of FAMILY covering CH (any coverage when `None`), resolving a
/// generic family through Fontconfig's aliases when it lists nothing itself.
pub(crate) fn list_family(family: &str, ch: Option<char>) -> Vec<ListedFace> {
    let faces = catalog::list_faces(Some(family), ch);
    if !faces.is_empty() || !is_generic_family(family) {
        return faces;
    }
    catalog::match_family(family)
        .map(|concrete| catalog::list_faces(Some(&concrete), ch))
        .unwrap_or_default()
}

fn score(
    face: &ListedFace,
    request: &FontRequest,
    compatibility: &impl Fn(&ListedFace) -> u32,
) -> crate::style::StyleScore {
    style_score(
        compatibility(face),
        request.weight,
        request.slant,
        request.width,
        face.weight,
        face.slant,
        face.width,
    )
}

fn best_face<'a>(
    faces: &'a [ListedFace],
    request: &FontRequest,
    order: FontSelectionState,
    compatibility: impl Fn(&ListedFace) -> u32,
) -> Option<&'a ListedFace> {
    faces
        .iter()
        .enumerate()
        .min_by_key(|(index, face)| (score(face, request, &compatibility).ordered(order), *index))
        .map(|(_, face)| face)
}
