//! A fontset as plain Rust data.
//!
//! Mirrors the inherited registry's lookup structure
//! (`romacs-core/src/emacs_core/display/fontset/mod.rs`, `FontsetData`):
//! sorted, non-overlapping character ranges, each holding an ordered list of
//! font specs, plus one fallback list consulted after the range's own
//! entries. Names are owned strings, not interned symbols, so a policy can be
//! captured once per generation and handed to any thread.
//!
//! What this type does not do, by design: expand a script symbol such as
//! `greek` into ranges. GNU's `Fset_fontset_font` does that against
//! `char-script-table` when the rule is written; a runtime publishing a
//! policy does the same and inserts the resulting ranges here.

/// One font alternative for a character range.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontsetFontSpec {
    /// `None` inherits the face's family, as a GNU font-spec without
    /// `:family` does.
    pub family: Option<String>,
    pub weight: Option<u16>,
    pub slant: Option<romacs_display_protocol::font::FontSlantKind>,
}

impl FontsetFontSpec {
    pub fn family(family: impl Into<String>) -> Self {
        Self {
            family: Some(family.into()),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FontsetEntry {
    Font(FontsetFontSpec),
    /// `(set-fontset-font FONTSET TARGET nil)`: the range has no font of its
    /// own. Resolution moves on (fallback list, face font, platform), as
    /// GNU's display does; the inherited core resolver instead stops here.
    ExplicitNone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontsetTarget {
    /// Inclusive scalar range.
    Range(u32, u32),
    /// The fontset's fallback list (`TARGET` nil in GNU).
    Fallback,
}

/// `set-fontset-font`'s ADD argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontsetMode {
    /// ADD nil: replace the target's entries.
    Overwrite,
    Prepend,
    Append,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FontsetRange {
    from: u32,
    to: u32,
    entries: Vec<FontsetEntry>,
}

/// An immutable-by-convention fontset snapshot with a change generation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontsetPolicy {
    ranges: Vec<FontsetRange>,
    fallback: Vec<FontsetEntry>,
    /// `face-font-family-alternatives`: family → ordered alternatives.
    family_alternatives: Vec<(String, Vec<String>)>,
    generation: u64,
}

impl FontsetPolicy {
    /// An empty fontset: every character resolves through the face's own
    /// font or the platform's generic fallback.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bumped by every edit. Resolver caches are keyed on it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Apply one `set-fontset-font` edit, with the inherited registry's
    /// range-splitting semantics.
    pub fn set_font(&mut self, target: FontsetTarget, entry: FontsetEntry, mode: FontsetMode) {
        match target {
            FontsetTarget::Fallback => {
                self.fallback = apply(Some(&self.fallback), entry, mode);
            }
            FontsetTarget::Range(from, to) => {
                let (from, to) = (from.min(to), from.max(to).min(u32::from(char::MAX)));
                self.update_range(from, to, &entry, mode);
            }
        }
        self.generation = self.generation.wrapping_add(1);
    }

    /// Set `face-font-family-alternatives` for FAMILY (compared
    /// case-insensitively, as GNU does).
    pub fn set_family_alternatives(
        &mut self,
        family: impl Into<String>,
        alternatives: Vec<String>,
    ) {
        let family = family.into();
        self.family_alternatives
            .retain(|(known, _)| !known.eq_ignore_ascii_case(&family));
        self.family_alternatives.push((family, alternatives));
        self.generation = self.generation.wrapping_add(1);
    }

    /// FAMILY followed by its alternatives, FAMILY first.
    pub(crate) fn families_for(&self, family: &str) -> Vec<String> {
        let mut out = vec![family.to_owned()];
        if let Some((_, alternatives)) = self
            .family_alternatives
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(family))
        {
            for alternative in alternatives {
                if !out
                    .iter()
                    .any(|seen| seen.eq_ignore_ascii_case(alternative))
                {
                    out.push(alternative.clone());
                }
            }
        }
        out
    }

    /// Entries for CH: the containing range's entries, then the fallback.
    pub fn entries_for(&self, ch: char) -> impl Iterator<Item = &FontsetEntry> {
        let code = u32::from(ch);
        let index = self.ranges.partition_point(|range| range.to < code);
        let specific = self
            .ranges
            .get(index)
            .filter(|range| range.from <= code)
            .map_or(&[][..], |range| range.entries.as_slice());
        specific.iter().chain(self.fallback.iter())
    }

    fn update_range(&mut self, from: u32, to: u32, entry: &FontsetEntry, mode: FontsetMode) {
        let mut next = Vec::with_capacity(self.ranges.len() + 2);
        let mut cursor = from;
        for range in std::mem::take(&mut self.ranges) {
            if range.to < from || range.from > to {
                if range.from > to && cursor <= to {
                    push(&mut next, cursor, to, apply(None, entry.clone(), mode));
                    cursor = to.saturating_add(1);
                }
                push(&mut next, range.from, range.to, range.entries);
                continue;
            }
            if range.from < from {
                push(&mut next, range.from, from - 1, range.entries.clone());
            }
            if cursor < range.from {
                push(
                    &mut next,
                    cursor,
                    range.from - 1,
                    apply(None, entry.clone(), mode),
                );
            }
            let overlap_from = range.from.max(from);
            let overlap_to = range.to.min(to);
            push(
                &mut next,
                overlap_from,
                overlap_to,
                apply(Some(&range.entries), entry.clone(), mode),
            );
            cursor = overlap_to.saturating_add(1);
            if range.to > to {
                push(&mut next, to + 1, range.to, range.entries);
            }
        }
        if cursor <= to && cursor >= from {
            push(&mut next, cursor, to, apply(None, entry.clone(), mode));
        }
        self.ranges = next;
    }
}

fn apply(
    existing: Option<&[FontsetEntry]>,
    entry: FontsetEntry,
    mode: FontsetMode,
) -> Vec<FontsetEntry> {
    match mode {
        FontsetMode::Overwrite => vec![entry],
        FontsetMode::Append => {
            let mut entries = existing.map(<[_]>::to_vec).unwrap_or_default();
            entries.push(entry);
            entries
        }
        FontsetMode::Prepend => {
            let mut entries = vec![entry];
            entries.extend(existing.into_iter().flatten().cloned());
            entries
        }
    }
}

fn push(ranges: &mut Vec<FontsetRange>, from: u32, to: u32, entries: Vec<FontsetEntry>) {
    if from <= to {
        ranges.push(FontsetRange { from, to, entries });
    }
}

#[cfg(test)]
#[path = "fontset/tests/fontset_test.rs"]
mod tests;
