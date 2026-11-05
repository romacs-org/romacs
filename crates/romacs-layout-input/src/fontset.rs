//! Fontsets as plain data: GNU's fontset.c rules, which the VM edits and the
//! font resolver reads, owned here because both depend on this crate.
//!
//! A fontset maps characters to an ordered list of font specifications, as
//! GNU's char-table of font-def vectors does, with one fallback list. The
//! table is persistent: [`Slot`] is a 64-way trie over the character space
//! and [`Alternatives`] a two-ended list, both sharing structure through
//! `Arc`, so an edit after a snapshot was published copies a few nodes, not
//! the rules it does not touch.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// An `:otf` property: (SCRIPT LANGSYS GSUB GPOS), each feature list holding
/// tags a font must have and, after a nil, tags it must lack.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct OtfSpec {
    pub script: String,
    pub langsys: Option<String>,
    pub gsub: Vec<(String, bool)>,
    pub gpos: Vec<(String, bool)>,
}
romacs_snapshot::snapshot_struct!(OtfSpec { script, langsys, gsub, gpos });

/// GNU's font-def: a font-spec reduced to what selection and the readers
/// use, with the charset `font-encoding-alist' gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontDef {
    pub family: Option<String>,
    pub foundry: Option<String>,
    pub registry: Option<String>,
    /// The spec's name, as `font-xlfd-name' gives it to `fontset-info'.
    pub name: String,
    /// CSS weight, an OS/2 width class, and 0 normal, 1 italic, 2 oblique:
    /// what a face's request holds.
    pub weight: Option<u16>,
    pub width: Option<u16>,
    pub slant: Option<u8>,
    /// `:size' in pixels; a face's own size when none.
    pub pixel_size: Option<u32>,
    pub lang: Vec<String>,
    /// The characters `:script' requires (`script-representative-chars').
    pub script_chars: Vec<u32>,
    pub otf: Option<OtfSpec>,
    /// FONT_DEF_ENCODING, its charset's id.
    pub encoding: u32,
}
romacs_snapshot::snapshot_struct!(FontDef { family, foundry, registry, name, weight, width, slant, pixel_size, lang, script_chars, otf, encoding });

impl FontDef {
    /// A def naming only FAMILY.
    pub fn family(family: impl Into<String>) -> Self {
        Self { family: Some(family.into()), ..Self::default() }
    }
}

/// One alternative: a font, or nil, which ends the search (fontset.c:635).
pub type Alternative = Option<Arc<FontDef>>;

struct Cell {
    entry: Alternative,
    next: Link,
}
romacs_snapshot::snapshot_struct!(Cell { entry, next });
type Link = Option<Arc<Cell>>;

/// Frees a long chain without recursing down it.
impl Drop for Cell {
    fn drop(&mut self) {
        let mut next = self.next.take();
        while let Some(cell) = next {
            match Arc::try_unwrap(cell) {
                Ok(mut cell) => next = cell.next.take(),
                Err(_) => break,
            }
        }
    }
}

/// An ordered list of alternatives: `front' in order, then `back' reversed.
/// Prepending and appending are O(1) and share all that came before.
#[derive(Clone, Default)]
pub struct Alternatives {
    front: Link,
    back: Link,
}
romacs_snapshot::snapshot_struct!(Alternatives { front, back });

impl std::fmt::Debug for Alternatives {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.to_vec()).finish()
    }
}

fn cons(entry: Alternative, next: &Link) -> Link {
    Some(Arc::new(Cell { entry, next: next.clone() }))
}

fn link_eq(a: &Link, b: &Link) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

impl Alternatives {
    pub fn one(entry: Alternative) -> Self {
        Self { front: cons(entry, &None), back: None }
    }

    pub fn prepended(&self, entry: Alternative) -> Self {
        Self { front: cons(entry, &self.front), back: self.back.clone() }
    }

    pub fn appended(&self, entry: Alternative) -> Self {
        Self { front: self.front.clone(), back: cons(entry, &self.back) }
    }

    /// The alternatives in order.
    pub fn to_vec(&self) -> Vec<Alternative> {
        let walk = |mut link: &Link, out: &mut Vec<Alternative>| {
            while let Some(cell) = link {
                out.push(cell.entry.clone());
                link = &cell.next;
            }
        };
        let (mut front, mut back) = (Vec::new(), Vec::new());
        walk(&self.front, &mut front);
        walk(&self.back, &mut back);
        back.reverse();
        front.extend(back);
        front
    }

    /// Whether SELF and OTHER are one list: the test for merging runs.
    fn same(&self, other: &Self) -> bool {
        link_eq(&self.front, &other.front) && link_eq(&self.back, &other.back)
    }
}

/// `set-fontset-font''s target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontsetTarget {
    /// Inclusive character range.
    Range(u32, u32),
    /// The fontset's fallback list (CHARACTERS nil).
    Fallback,
}

/// `set-fontset-font''s ADD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontsetMode {
    Overwrite,
    Prepend,
    Append,
}

const FANOUT: usize = 64;
/// The characters the root slot covers: 2^24, past MAX_CHAR.
const ROOT_SPAN: u32 = 1 << 24;

#[derive(Clone)]
enum Slot {
    Uniform(Option<Alternatives>),
    Node(Arc<[Slot; FANOUT]>),
}
romacs_snapshot::snapshot_enum!(Slot {
    Slot::Uniform(f0) => (f0),
    Slot::Node(f0) => (f0),
});

impl Default for Slot {
    fn default() -> Self { Slot::Uniform(None) }
}

fn added(old: Option<&Alternatives>, entry: &Alternative, mode: FontsetMode) -> Alternatives {
    match (mode, old) {
        (FontsetMode::Prepend, Some(old)) => old.prepended(entry.clone()),
        (FontsetMode::Append, Some(old)) => old.appended(entry.clone()),
        _ => Alternatives::one(entry.clone()),
    }
}

impl Slot {
    /// FROM..=TO in this slot, which covers SPAN characters from BASE, given
    /// ENTRY by MODE: touches only the nodes on the range's two ends and the
    /// slots it covers.
    fn add(&mut self, base: u32, span: u32, from: u32, to: u32, entry: &Alternative, mode: FontsetMode) {
        if from <= base && base + (span - 1) <= to {
            match self {
                Slot::Uniform(old) => *old = Some(added(old.as_ref(), entry, mode)),
                Slot::Node(_) if mode == FontsetMode::Overwrite => *self = Slot::Uniform(Some(added(None, entry, mode))),
                Slot::Node(node) => {
                    let child = span / FANOUT as u32;
                    for (i, slot) in Arc::make_mut(node).iter_mut().enumerate() {
                        slot.add(base + i as u32 * child, child, from, to, entry, mode);
                    }
                }
            }
            return;
        }
        if let Slot::Uniform(old) = self {
            let uniform = Slot::Uniform(old.clone());
            *self = Slot::Node(Arc::new(std::array::from_fn(|_| uniform.clone())));
        }
        let Slot::Node(node) = self else { unreachable!() };
        let child = span / FANOUT as u32;
        let first = (from.max(base) - base) / child;
        let last = (to.min(base + (span - 1)) - base) / child;
        let node = Arc::make_mut(node);
        for i in first..=last {
            node[i as usize].add(base + i * child, child, from, to, entry, mode);
        }
    }

    fn get(&self, code: u32) -> Option<&Alternatives> {
        let (mut slot, mut child) = (self, ROOT_SPAN / FANOUT as u32);
        loop {
            match slot {
                Slot::Uniform(alternatives) => return alternatives.as_ref(),
                Slot::Node(node) => {
                    slot = &node[(code / child) as usize % FANOUT];
                    child /= FANOUT as u32;
                }
            }
        }
    }

    /// Each run of this slot's characters holding one list, in order.
    fn runs(&self, base: u32, span: u32, visit: &mut impl FnMut(u32, u32, Option<&Alternatives>)) {
        match self {
            Slot::Uniform(alternatives) => visit(base, base + (span - 1), alternatives.as_ref()),
            Slot::Node(node) => {
                let child = span / FANOUT as u32;
                for (i, slot) in node.iter().enumerate() {
                    slot.runs(base + i as u32 * child, child, visit);
                }
            }
        }
    }
}

/// A base fontset.
#[derive(Clone, Default)]
pub struct Fontset {
    /// Its full name, and FONTSET_ASCII, the ASCII font's name.
    pub name: String,
    pub ascii: Option<String>,
    rules: Slot,
    fallback: Option<Alternatives>,
}
romacs_snapshot::snapshot_struct!(Fontset { name, ascii, rules, fallback });

impl Fontset {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Self::default() }
    }

    /// FONTSET_ADD: ENTRY for TARGET by MODE.
    pub fn add(&mut self, target: FontsetTarget, entry: Alternative, mode: FontsetMode) {
        match target {
            FontsetTarget::Fallback => self.fallback = Some(added(self.fallback.as_ref(), &entry, mode)),
            FontsetTarget::Range(from, to) => self.rules.add(0, ROOT_SPAN, from.min(to), from.max(to), &entry, mode),
        }
    }

    /// Fset_char_table_range (FONTSET, Qt, Qnil): no character has a rule.
    pub fn clear_rules(&mut self) {
        self.rules = Slot::default();
    }

    /// CH's alternatives in this fontset alone.
    pub fn get(&self, code: u32) -> Option<&Alternatives> {
        self.rules.get(code)
    }

    pub fn fallback(&self) -> Option<&Alternatives> {
        self.fallback.as_ref()
    }

    /// Each maximal run of characters with one list, in order, as
    /// char_table_ref_and_range walks the table.
    pub fn runs(&self, mut visit: impl FnMut(u32, u32, &Alternatives)) {
        let mut open: Option<(u32, u32, Alternatives)> = None;
        self.rules.runs(0, ROOT_SPAN, &mut |from, to, alternatives| {
            match (&mut open, alternatives) {
                (Some(run), Some(next)) if run.2.same(next) && run.1 + 1 == from => run.1 = to,
                (_, next) => {
                    if let Some((from, to, list)) = open.take() {
                        visit(from, to, &list);
                    }
                    open = next.map(|list| (from, to, list.clone()));
                }
            }
        });
        if let Some((from, to, list)) = open {
            visit(from, to, &list);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.fallback.is_none() && matches!(self.rules, Slot::Uniform(None))
    }
}

/// The font a def was realized to: what `fontset-info' names.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OpenedDefFont {
    pub family: String,
    pub file: String,
    pub selector: u32,
    pub pixel_size: u32,
}
romacs_snapshot::snapshot_struct!(OpenedDefFont { family, file, selector, pixel_size });

/// A def's identity in [`FontsetState::note_opened`]'s record.
pub fn def_key(def: &Arc<FontDef>) -> usize {
    Arc::as_ptr(def) as usize
}

/// What painting reads: the default fontset and the frame's, GNU's two
/// fontsets a face searches, and the charset order groups sort by. Publishing
/// it clones Arcs only; `revision' names it, so an equal revision is an
/// equal state.
#[derive(Clone, Default)]
pub struct FontsetState {
    pub default: Arc<Fontset>,
    pub frame: Arc<Fontset>,
    /// Vcharset_ordered_list up to Vcharset_non_preferred_head: each
    /// charset's position, and how many there are.
    pub charset_order: Arc<HashMap<u32, u32>>,
    pub preferred_charsets: u32,
    /// The `symbol' script's ranges, sorted, under
    /// `use-default-font-for-symbols': a face's own font comes first there.
    pub default_font_symbols: Arc<Vec<(u32, u32)>>,
    /// The fonts each def was realized to while this revision painted,
    /// shared by whoever holds the state: realized fontsets last as long.
    opened: Arc<Mutex<HashMap<usize, Vec<OpenedDefFont>>>>,
    pub revision: u64,
}
romacs_snapshot::snapshot_struct!(FontsetState { default, frame, charset_order, preferred_charsets, default_font_symbols, opened, revision });

impl std::fmt::Debug for FontsetState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("FontsetState").field("revision", &self.revision).finish()
    }
}

impl PartialEq for FontsetState {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
    }
}

impl FontsetState {
    /// A state with its own realization record, for REVISION.
    pub fn new(default: Arc<Fontset>, frame: Arc<Fontset>, charset_order: Arc<HashMap<u32, u32>>, preferred_charsets: u32, revision: u64) -> Self {
        Self { default, frame, charset_order, preferred_charsets, default_font_symbols: Arc::default(), opened: Arc::default(), revision }
    }

    /// Whether CODE is a symbol the face's own font takes first.
    pub fn default_font_first(&self, code: u32) -> bool {
        let ranges = &self.default_font_symbols;
        let index = ranges.partition_point(|&(_, to)| to < code);
        ranges.get(index).is_some_and(|&(from, _)| from <= code)
    }

    /// No rules at all: no Lisp runtime has written any.
    pub fn is_empty(&self) -> bool {
        self.default.is_empty() && self.frame.is_empty()
    }

    /// Record that DEF was realized to FONT.
    pub fn note_opened(&self, def: &Arc<FontDef>, font: OpenedDefFont) {
        let mut opened = self.opened.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let fonts = opened.entry(def_key(def)).or_default();
        if !fonts.contains(&font) {
            fonts.push(font);
        }
    }

    /// The fonts DEF was realized to.
    pub fn opened(&self, def: &Arc<FontDef>) -> Vec<OpenedDefFont> {
        self.opened.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&def_key(def)).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(family: &str) -> Alternative {
        Some(Arc::new(FontDef::family(family)))
    }

    fn names(alternatives: Option<&Alternatives>) -> Vec<String> {
        alternatives.map_or_else(Vec::new, |list| list.to_vec().iter().map(|entry| entry.as_ref().map_or("nil".to_string(), |def| def.family.clone().unwrap())).collect())
    }

    #[test]
    fn an_edit_splits_only_what_it_covers_and_later_rules_add_in_place() {
        let mut set = Fontset::new("f");
        set.add(FontsetTarget::Range(0x370, 0x3FF), def("A"), FontsetMode::Overwrite);
        set.add(FontsetTarget::Range(0x3A0, 0x3AF), def("B"), FontsetMode::Prepend);
        set.add(FontsetTarget::Range(0x3A8, 0x400), def("C"), FontsetMode::Append);
        assert_eq!(names(set.get(0x370)), ["A"]);
        assert_eq!(names(set.get(0x3A0)), ["B", "A"]);
        assert_eq!(names(set.get(0x3A8)), ["B", "A", "C"]);
        assert_eq!(names(set.get(0x3FF)), ["A", "C"]);
        assert_eq!(names(set.get(0x400)), ["C"]);
        assert_eq!(names(set.get(0x401)), Vec::<String>::new());
        assert_eq!(names(set.get(0x36F)), Vec::<String>::new());
    }

    #[test]
    fn overwrite_replaces_and_nil_is_an_alternative() {
        let mut set = Fontset::new("f");
        set.add(FontsetTarget::Range(0, 0x3FFFFF), def("A"), FontsetMode::Overwrite);
        set.add(FontsetTarget::Range(0x4E00, 0x9FFF), def("B"), FontsetMode::Append);
        set.add(FontsetTarget::Range(0, 0x3FFFFF), None, FontsetMode::Overwrite);
        assert_eq!(names(set.get(0x4E00)), ["nil"]);
        set.add(FontsetTarget::Fallback, def("F"), FontsetMode::Append);
        set.add(FontsetTarget::Fallback, def("G"), FontsetMode::Prepend);
        assert_eq!(names(set.fallback()), ["G", "F"]);
    }

    #[test]
    fn an_edit_leaves_a_published_copy_alone() {
        let mut set = Fontset::new("f");
        set.add(FontsetTarget::Range(0x41, 0x5A), def("A"), FontsetMode::Overwrite);
        let published = set.clone();
        set.add(FontsetTarget::Range(0x41, 0x41), def("B"), FontsetMode::Prepend);
        assert_eq!(names(published.get(0x41)), ["A"]);
        assert_eq!(names(set.get(0x41)), ["B", "A"]);
    }

    #[test]
    fn runs_merge_what_one_rule_wrote() {
        let mut set = Fontset::new("f");
        set.add(FontsetTarget::Range(0x100, 0x20000), def("A"), FontsetMode::Overwrite);
        set.add(FontsetTarget::Range(0x300, 0x301), def("B"), FontsetMode::Overwrite);
        let mut runs = Vec::new();
        set.runs(|from, to, list| runs.push((from, to, names(Some(list)))));
        assert_eq!(runs, [(0x100, 0x2FF, vec!["A".to_string()]), (0x300, 0x301, vec!["B".to_string()]), (0x302, 0x20000, vec!["A".to_string()])]);
    }

    #[test]
    fn a_long_list_is_dropped_without_recursion() {
        let mut list = Alternatives::one(None);
        for _ in 0..200_000 {
            list = list.prepended(None);
        }
        drop(list);
    }
}
