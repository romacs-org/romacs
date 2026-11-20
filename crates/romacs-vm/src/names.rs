//! The initial obarray's name table: each interned name's slot, keyed by the
//! name's internal multibyte bytes, so a Lisp string is looked up by its own
//! bytes and a name makes text only when it makes a symbol.
use crate::NameMap;
use std::borrow::Cow;

#[derive(Default, romacs_snapshot::Snapshot)]
pub(crate) struct Names {
    /// By a name's internal multibyte bytes (`key_of`, `text_key`), its slot.
    slots: NameMap<Box<[u8]>, usize>,
}

/// The key of a Lisp name given as BYTES of CHARS characters: its internal
/// multibyte bytes. A multibyte or all-ASCII name is its own bytes; a
/// unibyte name with a byte above 127 names raw-byte characters, as GNU's
/// `oblookup` tells it apart by its character count, so the unibyte bytes
/// C3 A9 (two characters) are not the multibyte "é" (one).
pub(crate) fn key_of(bytes: &[u8], chars: usize) -> Cow<'_, [u8]> {
    if chars < bytes.len() || bytes.is_ascii() { Cow::Borrowed(bytes) } else { Cow::Owned(crate::text::octets_to_multibyte(bytes)) }
}

/// The key of a name given as key text (`symkey`): its own bytes when it is
/// plain Unicode text, else the internal bytes its escapes stand for.
pub(crate) fn text_key(name: &str) -> Box<[u8]> {
    crate::symkey::to_internal(name).map_or_else(|| name.as_bytes().into(), Vec::into_boxed_slice)
}

impl Names {
    /// The slot of the name with key text NAME, when it is interned. An ASCII
    /// name, as nearly every name a Rust caller gives, is its own key.
    #[inline]
    pub(crate) fn get(&self, name: &str) -> Option<&usize> {
        if name.is_ascii() { self.slots.get(name.as_bytes()) } else { self.get_text(name) }
    }
    fn get_text(&self, name: &str) -> Option<&usize> {
        match crate::symkey::to_internal(name) {
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
        self.slots.insert(key, slot)
    }
    /// Forget the name with key text NAME.
    pub(crate) fn remove(&mut self, name: &str) {
        self.slots.remove(&*text_key(name));
    }
    /// Forget every name: the slots they named.
    pub(crate) fn take_slots(&mut self) -> Vec<usize> {
        self.slots.drain().map(|(_, slot)| slot).collect()
    }
    pub(crate) fn clear(&mut self) {
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
}
