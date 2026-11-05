//! Plain-data face and fontset → exact resolved fonts.

#[cfg(test)]
#[path = "resolver/tests/selection_order_test.rs"]
mod selection_order_tests;
#[cfg(test)]
#[path = "resolver/tests/alternative_state_test.rs"]
mod alternative_state_tests;
#[cfg(test)]
#[path = "resolver/tests/fontset_test.rs"]
mod fontset_tests;

use crate::instance::{FontInstanceInterner, FontInstanceProperties};
use crate::style::{FontRequest, RequestKey, style_score};
use romacs_layout_input::{Alternatives, FontAlternativeState, FontDef, FontSelectionAttribute, FontSelectionState, FontsetState, OpenedDefFont};
use std::sync::Arc;

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
pub(crate) use crate::fontconfig_catalog::{self as catalog, FaceList, ListSpec, ListedFace};

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
    #[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
    pub(crate) struct ListSpec {
        pub family: Option<String>,
        pub foundry: Option<String>,
        pub chars: Vec<u32>,
        pub langs: Vec<String>,
    }
    pub(crate) struct FaceList;
    impl FaceList {
        pub fn covering(&self, _: char) -> Vec<ListedFace> {
            Vec::new()
        }
    }
    pub(crate) fn list_constrained(_: &ListSpec) -> Option<FaceList> {
        None
    }
}
#[cfg(not(target_os = "linux"))]
use catalog::{FaceList, ListSpec, ListedFace};

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

    /// CH's advance and how far its ink rises above and falls below the
    /// baseline, in whole pixels (ftcrfont_glyph_extents: the ascent
    /// floored up from the bearing, the descent ceiled).
    fn ink(&self, ch: char) -> Option<(f32, f32, f32)> {
        let (_, advance) = self.glyph(ch)?;
        match self {
            #[cfg(target_os = "linux")]
            Self::FreeType(face) => {
                let metrics = face.glyph().metrics();
                let (top, height) = (metrics.horiBearingY as f32 / 64.0, metrics.height as f32 / 64.0);
                Some((advance, top.ceil(), (height - top).ceil()))
            }
            Self::Sfnt { data, index, pixel_size } => {
                let face = ttf_parser::Face::parse(data, *index).ok()?;
                let scale = pixel_size / f32::from(face.units_per_em());
                let bounds = face.glyph_index(ch).and_then(|glyph| face.glyph_bounding_box(glyph));
                Some(bounds.map_or((advance, 0.0, 0.0), |b| (advance, (f32::from(b.y_max) * scale).ceil(), (-f32::from(b.y_min) * scale).ceil())))
            }
        }
    }
}

/// GNU's `text_extents' for a run of characters in one font: the sum of
/// their advances and the most any one's ink rises above and falls below
/// the baseline, in whole pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InkExtents {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
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
        underline: face.underline_metrics().map_or_else(Default::default, |line| {
            romacs_display_protocol::font::FontUnderline::scaled(px(-f32::from(line.position)), px(f32::from(line.thickness)))
        }),
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
    fontsets: Arc<FontsetState>,
    primary: HashMap<RequestKey, Result<ResolvedFont, FontResolveError>>,
    chars: HashMap<(RequestKey, char), Result<CharResolution, FontResolveError>>,
    /// Romacs's icons: by the request's family, so a lookup allocates no
    /// key, then character, style and size (`resolve_sole').
    sole: HashMap<String, HashMap<(char, u16, romacs_display_protocol::font::FontSlantKind, u16, u32), Result<CharResolution, FontResolveError>>>,
    /// Fontconfig's listings for each spec, kept across rule changes: they
    /// depend on the installed fonts alone.
    lists: HashMap<ListSpec, Option<Arc<FaceList>>>,
    /// The fonts opened for each request and def so far, tried first
    /// (fontset_find_font's "font already opened").
    realized: HashMap<(RequestKey, usize), Vec<ResolvedFont>>,
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
            fontsets: Arc::default(),
            primary: HashMap::default(),
            chars: HashMap::default(),
            sole: HashMap::default(),
            lists: HashMap::default(),
            realized: HashMap::default(),
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
            self.retire_answers();
        }
    }

    /// Forget every answer that depends on rules or selection state.
    fn retire_answers(&mut self) {
        self.primary.clear();
        self.chars.clear();
        self.sole.clear();
        self.realized.clear();
    }

    /// Adopt the producer's fontsets: a new revision retires every answer.
    pub fn set_fontsets(&mut self, state: Arc<FontsetState>) {
        if *self.fontsets != *state {
            self.retire_answers();
        }
        self.fontsets = state;
    }

    pub fn alternative_state(&self) -> &FontAlternativeState { &self.alternatives }

    /// Adopt the producer's immutable family alist and setter history.
    pub fn set_alternative_state(&mut self, state: FontAlternativeState) {
        if self.alternatives != state {
            self.alternatives = state;
            self.retire_answers();
        }
    }

    /// Change style priorities and discard answers selected under the old order.
    /// Opened font files and interned exact instances remain reusable.
    pub fn set_selection_order(&mut self, order: [FontSelectionAttribute; 4]) {
        let mut state = self.selection_order;
        state.set_order(order);
        self.set_selection_state(state);
    }

    /// FAMILY, then its `face-font-family-alternatives' row.
    fn families_for(&self, family: &str) -> Vec<String> {
        let mut out = vec![family.to_owned()];
        if let Some(row) = self.alternatives.family_alist().iter().find(|row| row.first().is_some_and(|name| name.eq_ignore_ascii_case(family))) {
            for alternative in &row[1..] {
                if !out.iter().any(|seen| seen.eq_ignore_ascii_case(alternative)) {
                    out.push(alternative.clone());
                }
            }
        }
        out
    }

    /// The face's own font: the best-scoring scalable face of the request's
    /// family, then of each `face-font-family-alternatives` entry.
    pub fn resolve_primary(&mut self, request: &FontRequest) -> Result<ResolvedFont, FontResolveError> {
        let key = request.cache_key();
        if let Some(cached) = self.primary.get(&key) {
            return cached.clone();
        }
        let resolved = self.resolve_primary_uncached(request);
        self.primary.insert(key, resolved.clone());
        resolved
    }

    fn resolve_primary_uncached(&mut self, request: &FontRequest) -> Result<ResolvedFont, FontResolveError> {
        for family in self.families_for(&request.family) {
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

    /// The font for one non-ASCII character drawn in REQUEST's face, in
    /// fontset_font's order: the frame fontset's group for CH, the default
    /// fontset's, then their fallback groups, each skipped when the group
    /// before it in the same fontset stopped. Without any rule at all (no
    /// Lisp runtime), the face's font, then any installed font with the glyph.
    pub fn resolve_char(&mut self, request: &FontRequest, ch: char) -> Result<CharResolution, FontResolveError> {
        let key = (request.cache_key(), ch);
        if let Some(cached) = self.chars.get(&key) {
            return cached.clone();
        }
        let resolved = self.resolve_char_uncached(request, ch);
        self.chars.insert(key, resolved.clone());
        resolved
    }

    /// CH's font and glyph for a character of Romacs's icon font
    /// (U+FE000-U+FE0FF): resolved once per family, style and size through
    /// `resolve_char', then by a lookup that clones no font or family.
    pub fn resolve_sole(&mut self, request: &FontRequest, ch: char) -> Result<&CharResolution, FontResolveError> {
        let key = (ch, request.weight, request.slant, request.width, request.pixel_size.to_bits());
        if !self.sole.get(&request.family).is_some_and(|family| family.contains_key(&key)) {
            let resolved = self.resolve_char(request, ch);
            self.sole.entry(request.family.clone()).or_default().insert(key, resolved);
        }
        self.sole[&request.family][&key].as_ref().map_err(Clone::clone)
    }

    fn resolve_char_uncached(&mut self, request: &FontRequest, ch: char) -> Result<CharResolution, FontResolveError> {
        let state = Arc::clone(&self.fontsets);
        // face_for_char: ASCII is the face's own font.
        if state.is_empty() || ch.is_ascii() {
            return self.resolve_without_rules(request, ch);
        }
        let code = u32::from(ch);
        let ascii = self.resolve_primary(request).ok();
        // face_for_char: `use-default-font-for-symbols'.
        if state.default_font_first(code)
            && let Some(font) = &ascii
            && let Some((glyph_id, advance_px)) = self.glyph_in(font, ch) {
            return Ok(CharResolution {
                glyph: ResolvedCharGlyph { resolved_font_id: font.id, glyph_id, advance_px },
                font: font.clone(),
                source: CharResolutionSource::Primary,
            });
        }
        let own = self.search_group(state.frame.get(code), request, ch, ascii.as_ref(), &state)?;
        if let Group::Found(found) = own {
            return Ok(*found);
        }
        let separate = !Arc::ptr_eq(&state.frame, &state.default);
        let shared = if separate { self.search_group(state.default.get(code), request, ch, ascii.as_ref(), &state)? } else { Group::Missed };
        if let Group::Found(found) = shared {
            return Ok(*found);
        }
        if !matches!(own, Group::Stopped)
            && let Group::Found(found) = self.search_group(state.frame.fallback(), request, ch, ascii.as_ref(), &state)? {
            return Ok(*found);
        }
        if separate && !matches!(shared, Group::Stopped)
            && let Group::Found(found) = self.search_group(state.default.fallback(), request, ch, ascii.as_ref(), &state)? {
            return Ok(*found);
        }
        Err(FontResolveError::NoCoveringFont { code })
    }

    fn resolve_without_rules(&mut self, request: &FontRequest, ch: char) -> Result<CharResolution, FontResolveError> {
        let primary = self.resolve_primary(request)?;
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
        Err(FontResolveError::NoCoveringFont { code: u32::from(ch) })
    }

    /// fontset_find_font: the alternatives of a group in reorder_font_vector's
    /// order, until a font has CH or a nil stops the search.
    fn search_group(&mut self, group: Option<&Alternatives>, request: &FontRequest, ch: char, ascii: Option<&ResolvedFont>, state: &Arc<FontsetState>) -> Result<Group, FontResolveError> {
        let Some(group) = group else { return Ok(Group::Missed) };
        for (index, alternative) in reordered(group.to_vec(), ascii, state, |font, chars| self.has_all(font, chars)).into_iter().enumerate() {
            let Some(def) = alternative else { return Ok(Group::Stopped) };
            if let Some(found) = self.search_def(&def, request, ch, state)? {
                return Ok(Group::Found(Box::new(CharResolution { source: CharResolutionSource::Fontset(index), ..found })));
            }
        }
        Ok(Group::Missed)
    }

    fn has_all(&mut self, font: &ResolvedFont, chars: &[u32]) -> bool {
        chars.iter().all(|&code| char::from_u32(code).is_some_and(|ch| self.glyph_in(font, ch).is_some()))
    }

    /// A font for DEF having CH, as font_find_for_lface looks: the def's
    /// family, else the face's, its alternatives and any family, under the
    /// pattern ftfont_spec_pattern builds. A font already opened for this
    /// face and def is tried first.
    fn search_def(&mut self, def: &Arc<FontDef>, request: &FontRequest, ch: char, state: &FontsetState) -> Result<Option<CharResolution>, FontResolveError> {
        let key = (request.cache_key(), romacs_layout_input::fontset::def_key(def));
        for font in self.realized.get(&key).cloned().unwrap_or_default() {
            if let Some((glyph_id, advance_px)) = self.glyph_in(&font, ch) {
                return Ok(Some(CharResolution {
                    glyph: ResolvedCharGlyph { resolved_font_id: font.id, glyph_id, advance_px },
                    font,
                    source: CharResolutionSource::Fallback,
                }));
            }
        }
        let Some((chars, registry_lang)) = registry_constraints(def) else { return Ok(None) };
        let mut langs = def.lang.clone();
        langs.extend(registry_lang.map(str::to_owned));
        let styled = FontRequest {
            family: request.family.clone(),
            weight: def.weight.unwrap_or(request.weight),
            slant: def.slant.map_or(request.slant, slant_kind),
            width: def.width.unwrap_or(request.width),
            pixel_size: def.pixel_size.map_or(request.pixel_size, |size| size as f32),
        };
        let families: Vec<Option<String>> = match &def.family {
            Some(family) => vec![Some(family.clone())],
            None => self.families_for(&request.family).into_iter().map(Some).chain([None]).collect(),
        };
        for family in families {
            let spec = ListSpec { family, foundry: def.foundry.clone(), chars: chars.clone(), langs: langs.clone() };
            let Some(list) = self.listing(spec) else { continue };
            let mut faces = list.covering(ch);
            if let Some(otf) = &def.otf {
                faces.retain(|face| otf_matches(otf, face));
            }
            if let Some(found) = self.first_with_glyph(&faces, &styled, ch, |_| 0)? {
                state.note_opened(def, OpenedDefFont {
                    family: found.font.family.clone(),
                    file: found.font.identity.file_path.clone().unwrap_or_default(),
                    selector: found.font.identity.freetype_selector().unwrap_or(0),
                    pixel_size: found.font.pixel_size.round() as u32,
                });
                self.realized.entry(key).or_default().push(found.font.clone());
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    /// The listing for SPEC, resolving a generic family through Fontconfig's
    /// aliases when it lists nothing itself, once per spec.
    fn listing(&mut self, spec: ListSpec) -> Option<Arc<FaceList>> {
        if let Some(cached) = self.lists.get(&spec) {
            return cached.clone();
        }
        let generic = spec.family.as_deref().filter(|family| is_generic_family(family)).and_then(catalog::match_family);
        let concrete = ListSpec { family: generic.or_else(|| spec.family.clone()), ..spec.clone() };
        let list = catalog::list_constrained(&concrete).map(Arc::new);
        self.lists.insert(spec, list.clone());
        list
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

    /// CH's glyph in FONT and its advance, or None when FONT lacks it or
    /// cannot be opened.
    pub fn glyph_in_font(&mut self, font: &ResolvedFont, ch: char) -> Option<(ResolvedGlyphId, f32)> {
        self.glyph_in(font, ch)
    }

    /// TEXT's extents in FONT (font->driver->text_extents), or None when
    /// the font cannot be opened or lacks one of its glyphs.
    pub fn ink_extents(&mut self, font: &ResolvedFont, text: &str) -> Option<InkExtents> {
        let file = font.identity.file_path.clone()?;
        let selector = font.identity.freetype_selector()?;
        let face = self.opened_face(&file, selector, font.pixel_size)?;
        text.chars().try_fold(InkExtents::default(), |sum, ch| {
            let (advance, ascent, descent) = face.ink(ch)?;
            Some(InkExtents { width: sum.width + advance, ascent: sum.ascent.max(ascent), descent: sum.descent.max(descent) })
        })
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
                underline: metrics.underline,
            }))
    }
}

/// What a group's search came to.
enum Group {
    Found(Box<CharResolution>),
    /// A nil alternative: no other font for this character in this fontset.
    Stopped,
    Missed,
}

fn slant_kind(slant: u8) -> romacs_display_protocol::font::FontSlantKind {
    use romacs_display_protocol::font::FontSlantKind;
    match slant {
        1 => FontSlantKind::Italic,
        2 => FontSlantKind::Oblique,
        _ => FontSlantKind::Normal,
    }
}

/// ftfont.c's fc_charset_table: a registry's uniquifying characters and
/// language.
const REGISTRY_CHARSETS: &[(&str, &[u32], Option<&str>)] = &[
    ("iso8859-1", &[0x00A0, 0x00A1, 0x00B4, 0x00BC, 0x00D0], None),
    ("iso8859-2", &[0x00A0, 0x010E], None),
    ("iso8859-3", &[0x00A0, 0x0108], None),
    ("iso8859-4", &[0x00A0, 0x00AF, 0x0128, 0x0156, 0x02C7], None),
    ("iso8859-5", &[0x00A0, 0x0401], None),
    ("iso8859-6", &[0x00A0, 0x060C], None),
    ("iso8859-7", &[0x00A0, 0x0384], None),
    ("iso8859-8", &[0x00A0, 0x05D0], None),
    ("iso8859-9", &[0x00A0, 0x00A1, 0x00BC, 0x011E], None),
    ("iso8859-10", &[0x00A0, 0x00D0, 0x0128, 0x2015], None),
    ("iso8859-11", &[0x00A0, 0x0E01], None),
    ("iso8859-13", &[0x00A0, 0x201C], None),
    ("iso8859-14", &[0x00A0, 0x0174], None),
    ("iso8859-15", &[0x00A0, 0x00A1, 0x00D0, 0x0152], None),
    ("iso8859-16", &[0x00A0, 0x0218], None),
    ("gb2312.1980-0", &[0x4E13], Some("zh-cn")),
    ("big5-0", &[0x9C21], Some("zh-tw")),
    ("jisx0208.1983-0", &[0x4E55], Some("ja")),
    ("ksc5601.1985-0", &[0xAC00], Some("ko")),
    ("cns11643.1992-1", &[0xFE32], Some("zh-tw")),
    ("cns11643.1992-2", &[0x4E33, 0x7934], None),
    ("cns11643.1992-3", &[0x201A9], None),
    ("cns11643.1992-4", &[0x20057], None),
    ("cns11643.1992-5", &[0x20000], None),
    ("cns11643.1992-6", &[0x20003], None),
    ("cns11643.1992-7", &[0x20055], None),
    ("gbk-0", &[0x4E06], Some("zh-cn")),
    ("jisx0212.1990-0", &[0x4E44], None),
    ("jisx0213.2000-1", &[0xFA10], Some("ja")),
    ("jisx0213.2000-2", &[0xFA49], None),
    ("jisx0213.2004-1", &[0x20B9F], None),
    ("viscii1.1-1", &[0x1EA0, 0x1EAE, 0x1ED2], Some("vi")),
    ("tis620.2529-1", &[0x0E01], Some("th")),
    ("microsoft-cp1251", &[0x0401, 0x0490], Some("ru")),
    ("koi8-r", &[0x0401, 0x2219], Some("ru")),
    ("mulelao-1", &[0x0E81], Some("lo")),
    ("unicode-sip", &[0x20000], None),
];

/// Whether PATTERN, with `*' any run and `?' any one character, occurs in
/// TEXT, ignoring case: ftfont_get_charset's regexp.
fn registry_matches(pattern: &[u8], text: &[u8]) -> bool {
    fn at(pattern: &[u8], text: &[u8]) -> bool {
        match pattern.split_first() {
            None => true,
            Some((b'*', rest)) => (0..=text.len()).any(|skip| at(rest, &text[skip..])),
            Some((&p, rest)) => text.split_first().is_some_and(|(&t, tail)| (p == b'?' || p.eq_ignore_ascii_case(&t)) && at(rest, tail)),
        }
    }
    (0..=text.len()).any(|start| at(pattern, &text[start..]))
}

/// The characters and language DEF requires of a font, as
/// ftfont_spec_pattern finds them; None for a registry no font can have.
fn registry_constraints(def: &FontDef) -> Option<(Vec<u32>, Option<&'static str>)> {
    match def.registry.as_deref() {
        None | Some("ascii-0" | "iso10646-1" | "unicode-bmp") => Some((def.script_chars.clone(), None)),
        Some(registry) => REGISTRY_CHARSETS.iter().find(|(name, ..)| registry_matches(registry.as_bytes(), name.as_bytes()))
            .map(|&(_, chars, lang)| (chars.to_vec(), lang)),
    }
}

/// ftfont_check_otf: FACE's tables have OTF's script and language system,
/// with each wanted feature and no unwanted one.
fn otf_matches(otf: &romacs_layout_input::OtfSpec, face: &ListedFace) -> bool {
    let Some(capability) = crate::probe::otf_capability(&face.file, face.selector & 0xffff) else { return false };
    let table_has = |table: &[crate::probe::OtfScript], wanted: &[(String, bool)]| {
        let Some(script) = table.iter().find(|script| script.tag == otf.script) else { return wanted.is_empty() };
        let features = |lang: &crate::probe::OtfLangSys| wanted.iter().all(|(tag, present)| lang.features.contains(tag) == *present);
        script.lang_syses.iter().filter(|lang| otf.langsys.is_none() || lang.tag == otf.langsys).any(features)
    };
    let scripted = capability.gsub.iter().chain(&capability.gpos).any(|script| script.tag == otf.script);
    scripted && table_has(&capability.gsub, &otf.gsub) && table_has(&capability.gpos, &otf.gpos)
}

/// reorder_font_vector: ALTERNATIVES in listing order, those matching the
/// face's ASCII font or naming `:otf' first and the rest by where their
/// encoding stands in the charset order. A nil at the end stays there; nil
/// elsewhere sorts first once anything moved, as fontset_compare_rfontdef.
fn reordered(alternatives: Vec<romacs_layout_input::Alternative>, ascii: Option<&ResolvedFont>, state: &FontsetState, mut has_all: impl FnMut(&ResolvedFont, &[u32]) -> bool) -> Vec<romacs_layout_input::Alternative> {
    if alternatives.len() < 2 {
        return alternatives;
    }
    let trailing = alternatives.last().is_some_and(Option::is_none);
    let scored = alternatives.len() - usize::from(trailing);
    let rank = |def: &FontDef| state.charset_order.get(&def.encoding).copied().unwrap_or(state.preferred_charsets);
    let mut keyed: Vec<(i64, romacs_layout_input::Alternative)> = alternatives.into_iter().enumerate().map(|(index, alternative)| {
        let score = match &alternative {
            _ if index >= scored => i64::MAX,
            None => i64::MIN,
            Some(def) if def.otf.is_some() || ascii.is_some_and(|font| def_matches(def, font, &mut has_all)) => index as i64,
            Some(def) => index as i64 + 0x100 * i64::from(rank(def)),
        };
        (score, alternative)
    }).collect();
    if keyed.iter().any(|(score, _)| (0x100..i64::MAX).contains(score)) {
        keyed.sort_by_key(|(score, _)| *score);
    }
    keyed.into_iter().map(|(_, alternative)| alternative).collect()
}

/// font_match_p of DEF against the face's ASCII font: every property the
/// def names is the font's. Listed fonts are all Unicode-encoded.
fn def_matches(def: &FontDef, font: &ResolvedFont, has_all: &mut impl FnMut(&ResolvedFont, &[u32]) -> bool) -> bool {
    def.family.as_ref().is_none_or(|family| family.eq_ignore_ascii_case(&font.family))
        && def.registry.as_ref().is_none_or(|registry| registry.eq_ignore_ascii_case("iso10646-1"))
        && def.weight.is_none_or(|weight| weight == font.weight)
        && def.slant.is_none_or(|slant| slant_kind(slant) == font.slant)
        && def.width.is_none_or(|width| width == font.width)
        && def.lang.is_empty()
        && has_all(font, &def.script_chars)
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
