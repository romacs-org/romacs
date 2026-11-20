//! The initial obarray's name table: each interned name's slot, keyed by the
//! name's internal multibyte bytes, so a Lisp string is looked up by its own
//! bytes and a name makes text only when it makes a symbol.
use std::hash::{BuildHasher, Hasher};
use std::borrow::Cow;

#[derive(Default, romacs_snapshot::Snapshot)]
pub(crate) struct Names {
    /// By a name's internal multibyte bytes (`key_of`, `text_key`), its slot.
    slots: std::collections::HashMap<Box<[u8]>, usize, KeyState>,
    /// Whether some key interned holds the escape's bytes (`symkey::ESC`):
    /// only such a key could equal, byte for byte, key text that escapes
    /// (`get`).
    escaped: bool,
}

/// The names table's hashing: `NameState`'s (a random seed per table) over
/// a key's bytes alone. A byte slice's `Hash` writes its length before its
/// bytes; the table compares keys whose hashes meet anyway, so the length
/// tells it nothing, and one write is what a lookup of a hit costs.
#[derive(Clone, Default, romacs_snapshot::Snapshot)]
pub(crate) struct KeyState(crate::NameState);

impl BuildHasher for KeyState {
    type Hasher = KeyHasher;
    #[inline]
    fn build_hasher(&self) -> KeyHasher {
        KeyHasher(self.0.build_hasher())
    }
}

/// `KeyState`'s hasher: a key's bytes, its length prefix left out.
pub(crate) struct KeyHasher(<crate::NameState as BuildHasher>::Hasher);

impl Hasher for KeyHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }
    /// A key's length prefix (`[u8]`'s `Hash`): left out.
    #[inline]
    fn write_usize(&mut self, _: usize) {}
    #[inline]
    fn finish(&self) -> u64 {
        self.0.finish()
    }
}

/// `symkey::ESC` in UTF-8.
const ESC_BYTES: [u8; 3] = [0xEF, 0xB7, 0x90];

/// The key of a Lisp name given as BYTES of CHARS characters. A multibyte
/// or all-ASCII name is its own bytes; a unibyte name with a byte above
/// 127 is its bytes behind 0xFF, which internal multibyte text never
/// holds, as GNU's `oblookup` tells names apart by character count and
/// bytes: the unibyte bytes C3 A9 (two characters) are not the multibyte
/// "é" (one), nor the unibyte E9 the multibyte raw byte E9.
pub(crate) fn key_of(bytes: &[u8], chars: usize) -> Cow<'_, [u8]> {
    if chars < bytes.len() || bytes.is_ascii() { Cow::Borrowed(bytes) } else { Cow::Owned([&[0xFF], bytes].concat()) }
}

/// The key of a name given as key text (`symkey`): its own bytes when it is
/// plain Unicode text, else the internal bytes its escapes stand for.
pub(crate) fn text_key(name: &str) -> Box<[u8]> {
    crate::symkey::names_key(name).map_or_else(|| name.as_bytes().into(), Vec::into_boxed_slice)
}

impl Names {
    /// The slot of the name with key text NAME, when it is interned. Key
    /// text without the escape is its own key, and key text with it can equal
    /// only a key holding the escape's bytes: unless one is interned, NAME's
    /// own bytes find it, and only a miss asks whether NAME escapes.
    #[inline(always)]
    pub(crate) fn get(&self, name: &str) -> Option<&usize> {
        if self.escaped { return self.get_text(name); }
        match self.slots.get(name.as_bytes()) {
            Some(slot) => Some(slot),
            None => self.get_escaped(name),
        }
    }
    /// NAME's slot when its own bytes found none: by the key its escapes
    /// stand for, if it has any.
    #[cold]
    #[inline(never)]
    fn get_escaped(&self, name: &str) -> Option<&usize> {
        if name.is_ascii() { return None; }
        self.slots.get(&crate::symkey::names_key(name)?[..])
    }
    #[cold]
    #[inline(never)]
    fn get_text(&self, name: &str) -> Option<&usize> {
        match crate::symkey::names_key(name) {
            Some(key) => self.slots.get(&key[..]),
            None => self.slots.get(name.as_bytes()),
        }
    }
    /// The slot of the name whose key (`key_of`) is KEY, when it is interned.
    pub(crate) fn get_bytes(&self, key: &[u8]) -> Option<&usize> {
        self.slots.get(key)
    }
    pub(crate) fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
    /// Make the name with key text NAME name SLOT: the slot it named before,
    /// as `HashMap::insert`.
    pub(crate) fn insert(&mut self, name: &str, slot: usize) -> Option<usize> {
        self.insert_key(text_key(name), slot)
    }
    /// Make the name whose key (`key_of`, `text_key`) is KEY name SLOT.
    pub(crate) fn insert_key(&mut self, key: Box<[u8]>, slot: usize) -> Option<usize> {
        self.escaped |= key.windows(ESC_BYTES.len()).any(|w| w == ESC_BYTES);
        self.slots.insert(key, slot)
    }
    /// Forget the name with key text NAME.
    pub(crate) fn remove(&mut self, name: &str) {
        self.slots.remove(&*text_key(name));
    }
    /// Forget every name: the slots they named.
    pub(crate) fn take_slots(&mut self) -> Vec<usize> {
        self.escaped = false;
        self.slots.drain().map(|(_, slot)| slot).collect()
    }
    pub(crate) fn clear(&mut self) {
        self.escaped = false;
        self.slots.clear();
    }
    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }
    /// The interned slots, in no order.
    pub(crate) fn slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.slots.values().copied()
    }
}

/// NAME's slot, as `HashMap`'s index: NAME must be interned.
impl std::ops::Index<&str> for Names {
    type Output = usize;
    fn index(&self, name: &str) -> &usize {
        self.get(name).unwrap_or_else(|| panic!("no symbol is named {name:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Names, key_of};

    #[test]
    fn a_name_is_keyed_by_its_internal_bytes_and_a_unibyte_one_by_its_raw_bytes() {
        let mut names = Names::default();
        names.insert("car", 1);
        names.insert("é", 2);
        assert_eq!(names.get_bytes(&key_of(b"car", 3)), Some(&1), "ASCII, unibyte or multibyte, is one name");
        assert_eq!(names.get_bytes(&key_of("é".as_bytes(), 1)), Some(&2), "multibyte é is its UTF-8");
        assert_eq!(names.get_bytes(&key_of(&[0xC3, 0xA9], 2)), None, "unibyte C3 A9 is two raw bytes, not é");
        assert_eq!(names.get("é"), Some(&2));
        names.remove("é");
        assert_eq!((names.get("é"), names.len()), (None, 1));
    }

    #[test]
    fn key_text_that_escapes_finds_its_name_and_never_one_whose_bytes_it_spells() {
        let mut names = Names::default();
        // A raw byte, as key text: U+FDD0 and two Plane 15 characters. Its
        // key holds no U+FDD0, so lookups stay on the direct path.
        let mut raw = String::new();
        crate::symkey::push_char(&mut raw, 0x3F_FF80);
        names.insert(&raw, 1);
        assert_eq!((names.get(&raw), names.escaped), (Some(&1), false), "found by the key its escape stands for");
        // A name spelt "a" and two genuine U+FDD0: its key is those bytes.
        let mut doubled = String::from("a");
        crate::symkey::push_char(&mut doubled, 0xFDD0);
        crate::symkey::push_char(&mut doubled, 0xFDD0);
        names.insert(&doubled, 2);
        assert_eq!((names.get(&doubled), names.escaped), (Some(&2), true));
        // The key text "a" + U+FDD0 + U+FDD0 names "a" and one U+FDD0, never
        // interned, though its own bytes are the doubled name's key.
        let single = "a\u{fdd0}\u{fdd0}";
        assert_eq!(names.get_bytes(single.as_bytes()), Some(&2), "the bytes do spell the doubled name's key");
        assert_eq!(names.get(single), None);
        assert_eq!(names.get(&raw), Some(&1));
    }
}
