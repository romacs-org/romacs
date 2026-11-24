//! Canonical records for metrics-bearing font instances.
//!
//! GNU `font_open_entity` reuses an opened entity at its pixel size when
//! `cached_font_ok` permits it; `font_clear_cache` invalidates its observations.
//! Here IDs survive for the resolver lifetime, while records expire only on a
//! catalog transition. Keeping both states here prevents independent builders
//! or cache-clearing paths from publishing conflicting records under one ID.

use romacs_display_protocol::font::{
    FontReplay, FontSlantKind, ResolvedFont, ResolvedFontAdvance, ResolvedFontId,
    ResolvedFontIdentity,
};
use romacs_display_protocol::pane_picture::{owner_slot_base, OWNER_SLOT_SPAN};
use rustc_hash::FxHashMap;

/// Builders provide observations, never the identity fields owned by the key.
/// In particular, a builder cannot choose another ID, replay, or pixel size.
pub struct FontInstanceProperties {
    pub family: String,
    pub full_name: Option<String>,
    pub postscript_name: Option<String>,
    pub weight: u16,
    pub slant: FontSlantKind,
    pub width: u16,
    pub ascent_px: f32,
    pub descent_px: f32,
    pub space_advance_px: f32,
    pub glyph_advance: ResolvedFontAdvance,
    pub underline: romacs_display_protocol::font::FontUnderline,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FontInstanceKey {
    identity: ResolvedFontIdentity,
    replay: FontReplay,
    pixel_size_bits: u32,
}

struct InternedFontInstance {
    id: ResolvedFontId,
    record: Option<ResolvedFont>,
}

/// Where an interner's ids start and the range they stay in: the
/// master's, below owner slot 0's first id, or one owner slot's
/// (`romacs_display_protocol::pane_picture::owner_slot_base`), so no two
/// owners and no owner and the master share a font id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontIdBase {
    first: u32,
    count: u32,
}

impl FontIdBase {
    /// The master's: ids 1 up to, not including, slot 0's first.
    pub const MASTER: Self = Self { first: 1, count: OWNER_SLOT_SPAN - 1 };

    /// Owner slot SLOT's: its slot's ids. None for a slot past the last.
    #[must_use]
    pub fn owner(slot: u8) -> Option<Self> {
        owner_slot_base(slot).map(|first| Self { first, count: OWNER_SLOT_SPAN })
    }

    /// The id of instance INDEX in order of interning (from 0), or None
    /// when it would leave the range.
    #[must_use]
    pub fn id(self, index: usize) -> Option<ResolvedFontId> {
        let index = u32::try_from(index).ok().filter(|&index| index < self.count)?;
        Some(ResolvedFontId(self.first + index))
    }
}

pub struct FontInstanceInterner {
    base: FontIdBase,
    instances: FxHashMap<FontInstanceKey, InternedFontInstance>,
}

impl Default for FontInstanceInterner {
    /// The master's interner.
    fn default() -> Self {
        Self::with_base(FontIdBase::MASTER)
    }
}

impl FontInstanceInterner {
    /// An interner whose ids are BASE's, in order of first interning.
    #[must_use]
    pub fn with_base(base: FontIdBase) -> Self {
        Self { base, instances: FxHashMap::default() }
    }

    /// The font IDENTITY, REPLAY and `pixel_size` name, under the id it was
    /// first interned with, or the next of the base's; its record observed
    /// by OBSERVE if the catalog changed since or it is new.
    ///
    /// # Panics
    ///
    /// When the base's ids are spent.
    pub fn intern(
        &mut self,
        identity: ResolvedFontIdentity,
        replay: FontReplay,
        pixel_size: f32,
        observe: impl FnOnce() -> FontInstanceProperties,
    ) -> ResolvedFont {
        let key = FontInstanceKey {
            identity: identity.clone(),
            replay: replay.clone(),
            pixel_size_bits: pixel_size.to_bits(),
        };
        let (allocated, base) = (self.instances.len(), self.base);
        let instance = self
            .instances
            .entry(key)
            .or_insert_with(|| InternedFontInstance {
                id: base.id(allocated).expect("resolved font ID space exhausted"),
                record: None,
            });
        instance
            .record
            .get_or_insert_with(|| {
                let properties = observe();
                ResolvedFont {
                    id: instance.id,
                    identity,
                    replay,
                    pixel_size,
                    family: properties.family,
                    full_name: properties.full_name,
                    postscript_name: properties.postscript_name,
                    weight: properties.weight,
                    slant: properties.slant,
                    width: properties.width,
                    ascent_px: properties.ascent_px,
                    descent_px: properties.descent_px,
                    space_advance_px: properties.space_advance_px,
                    glyph_advance: properties.glyph_advance,
                    underline: properties.underline,
                }
            })
            .clone()
    }

    /// Old frame snapshots retain their owned records; new observations reuse
    /// the stable ID after a catalog change. Ordinary cache clears do not call
    /// this, so request provenance cannot change a published record.
    pub fn invalidate_catalog_records(&mut self) {
        for instance in self.instances.values_mut() {
            instance.record = None;
        }
    }
}
