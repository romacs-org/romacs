//! O1b-1a commit 1a-2: an owner's fonts intern from its slot's base, so no
//! two owners and no owner and the master share a font id (design O1b.md
//! rev 3, §6.2 "Font slots").
//!
//! The interface these tests drive, in `romacs_font::instance`:
//!
//! ```ignore
//! /// Where an interner's ids start and the range they stay in.
//! #[derive(Clone, Copy, Debug, PartialEq, Eq)]
//! pub struct FontIdBase(..);
//! impl FontIdBase {
//!     /// The master's: ids 1 .. 1 << 24.
//!     pub const MASTER: Self;
//!     /// Owner slot SLOT's: ids (SLOT+1) << 24 .. (SLOT+2) << 24.
//!     /// None for 255, whose range would pass u32.
//!     pub fn owner(slot: u8) -> Option<Self>;
//!     /// The id of the INDEXth instance interned (from 0), or None when it
//!     /// would leave the range: the interner refuses it (it panics, as it
//!     /// does today on an exhausted id space).
//!     pub fn id(self, index: usize) -> Option<ResolvedFontId>;
//! }
//! impl FontInstanceInterner {
//!     /// `Default` stays `with_base(FontIdBase::MASTER)`.
//!     pub fn with_base(base: FontIdBase) -> Self;
//! }
//! ```
#![forbid(unsafe_code)]

use romacs_display_protocol::font::{
    FontFileAsset, FontOutlineAsset, FontReplay, FontSlantKind, ResolvedFontId,
    ResolvedFontIdentity,
};
use romacs_font::instance::{FontIdBase, FontInstanceInterner, FontInstanceProperties};

const SLOT_SPAN: usize = 1 << 24;

fn properties() -> FontInstanceProperties {
    FontInstanceProperties {
        family: "Test".into(),
        full_name: None,
        postscript_name: None,
        weight: 400,
        slant: FontSlantKind::Normal,
        width: 5,
        ascent_px: 11.0,
        descent_px: 3.0,
        space_advance_px: 8.0,
        glyph_advance: Default::default(),
        underline: Default::default(),
    }
}

/// The id INTERNER gives the font in FILE at 14 px.
fn intern(interner: &mut FontInstanceInterner, file: &str) -> ResolvedFontId {
    let replay = FontReplay::Swash { asset: FontOutlineAsset::File(FontFileAsset::new(file, 0).unwrap()) };
    interner.intern(ResolvedFontIdentity::from_file(file, 0, None), replay, 14.0, properties).id
}

#[test]
fn an_owner_interner_starts_at_its_slots_base() {
    let mut interner = FontInstanceInterner::with_base(FontIdBase::owner(3).unwrap());
    let first = intern(&mut interner, "/fonts/a.ttf");
    assert_eq!(first, ResolvedFontId(4 << 24));
    assert_eq!(intern(&mut interner, "/fonts/b.ttf"), ResolvedFontId((4 << 24) + 1));
    assert_eq!(intern(&mut interner, "/fonts/a.ttf"), first, "a font interned again keeps its id");
}

#[test]
fn the_master_interner_still_starts_at_one() {
    let mut master = FontInstanceInterner::default();
    assert_eq!(intern(&mut master, "/fonts/a.ttf"), ResolvedFontId(1));
    let mut explicit = FontInstanceInterner::with_base(FontIdBase::MASTER);
    assert_eq!(intern(&mut explicit, "/fonts/a.ttf"), ResolvedFontId(1));
}

#[test]
fn two_owner_slots_never_share_an_id() {
    let (zero, one) = (FontIdBase::owner(0).unwrap(), FontIdBase::owner(1).unwrap());
    let (mut a, mut b) = (FontInstanceInterner::with_base(zero), FontInstanceInterner::with_base(one));
    assert_ne!(intern(&mut a, "/fonts/a.ttf"), intern(&mut b, "/fonts/a.ttf"));
    // Slot 0's last id is just below slot 1's first, and slot 0 stops there.
    assert_eq!(zero.id(0), Some(ResolvedFontId(1 << 24)));
    assert_eq!(zero.id(SLOT_SPAN - 1), Some(ResolvedFontId((2 << 24) - 1)));
    assert_eq!(zero.id(SLOT_SPAN), None);
    assert_eq!(one.id(0), Some(ResolvedFontId(2 << 24)));
}

#[test]
fn slot_255_is_refused_and_254_ends_at_the_top_of_u32() {
    assert_eq!(FontIdBase::owner(255), None);
    let last = FontIdBase::owner(254).unwrap();
    assert_eq!(last.id(0), Some(ResolvedFontId(255 << 24)));
    assert_eq!(last.id(SLOT_SPAN - 1), Some(ResolvedFontId(u32::MAX)));
    assert_eq!(last.id(SLOT_SPAN), None);
}

#[test]
fn the_master_refuses_to_reach_the_first_slot() {
    assert_eq!(FontIdBase::MASTER.id(0), Some(ResolvedFontId(1)));
    assert_eq!(FontIdBase::MASTER.id(SLOT_SPAN - 2), Some(ResolvedFontId((1 << 24) - 1)));
    assert_eq!(FontIdBase::MASTER.id(SLOT_SPAN - 1), None, "id 1 << 24 is slot 0's");
}
