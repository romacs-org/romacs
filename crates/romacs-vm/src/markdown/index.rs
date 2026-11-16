//! A document's top-level block starts, kept across its edits, so a region
//! is parsed as the blocks that hold it and an edit reparses the blocks it
//! touched. Positions are character offsets into the document. The text is
//! read through a READ function: the index holds none of it.
use super::spans::spans;
use std::collections::HashMap;
use std::ops::Range;

/// The text of a range of characters.
pub(crate) type Read<'a> = dyn FnMut(Range<usize>) -> String + 'a;

#[derive(Clone, Debug, Default)]
pub(crate) struct Index {
    /// The top-level blocks' line starts, ascending; right outside `dirty`.
    starts: Vec<usize>,
    /// The hull of what changed since the starts were last right.
    dirty: Option<Range<usize>>,
    /// The document's link reference definitions: where each starts and the
    /// label it defines, ascending, as `starts` are kept.
    defs: Vec<(usize, String)>,
    /// How many definitions define each label.
    labels: HashMap<String, usize>,
    /// The document's length, in characters, as its edits leave it.
    len: usize,
    built: bool,
}
romacs_snapshot::snapshot_struct!(Index { starts, dirty, defs, labels, len, built });

impl Index {
    /// START..END of the document was replaced by INSERTED characters.
    pub(crate) fn edit(&mut self, start: usize, end: usize, inserted: usize) {
        if !self.built {
            return;
        }
        let new_end = start + inserted;
        let shift = |p: usize| if p >= end { p - end + new_end } else if p > start { new_end } else { p };
        self.starts.retain(|&s| s <= start || s > end);
        for s in &mut self.starts {
            *s = shift(*s);
        }
        for (at, _) in &mut self.defs {
            *at = shift(*at);
        }
        let changed = match self.dirty.take() {
            Some(d) => shift(d.start).min(start)..shift(d.end).max(new_end),
            None => start..new_end,
        };
        self.dirty = Some(changed);
        self.len = self.len + inserted - (end - start);
    }

    /// Whether the document has been read since this index was made.
    pub(crate) fn built(&self) -> bool {
        self.built
    }

    /// Whether a reference to LABEL (a `label_key`) resolves in the document.
    pub(crate) fn defines(&self, label: &str) -> bool {
        self.labels.get(label).is_some_and(|&n| n > 0)
    }

    /// Make the starts right for a document of LEN characters, reading it
    /// through READ: the whole once, then what changed and the blocks
    /// around it until an old start is found again. Returns the span read
    /// again, whose faces may have changed; None when nothing had.
    pub(crate) fn sync(&mut self, len: usize, read: &mut Read<'_>) -> Option<Range<usize>> {
        if !self.built || self.len != len {
            let text = read(0..len);
            let parsed = spans(&text);
            self.starts = char_starts(&text, &parsed.tops, 0);
            self.defs = defs(&text, parsed.labels, 0);
            self.labels = counts(&self.defs);
            (self.len, self.built, self.dirty) = (len, true, None);
            return Some(0..len);
        }
        let dirty = self.dirty.take()?;
        // From the block before the one the change starts in: what arrives
        // can join a block to the one above it.
        let held = self.starts.partition_point(|&s| s <= dirty.start);
        let keep = held.saturating_sub(2);
        let from = if held >= 2 { self.starts[keep] } else { 0 };
        // To an old start after the change found again; one past it first.
        let after = self.starts.partition_point(|&s| s <= dirty.end);
        let mut k = after + 1;
        loop {
            let cut = self.starts.get(k).copied();
            let end = self.starts.get(k + 1).copied().unwrap_or(len);
            let text = read(from..end);
            let parsed = spans(&text);
            let found = char_starts(&text, &parsed.tops, from);
            if cut.is_none_or(|c| found.contains(&c)) {
                let cut_at = cut.unwrap_or(len);
                let mut starts = self.starts[..keep].to_vec();
                starts.extend(found.into_iter().filter(|&s| s < cut_at));
                starts.extend_from_slice(&self.starts[k.min(self.starts.len())..]);
                self.starts = starts;
                // The definitions read again replace those in the span: a
                // label defined or no longer defined changes what every
                // reference to it is, so the whole document is again to face.
                self.defs.retain(|(at, _)| *at < from || cut.is_some_and(|c| *at >= c));
                self.defs.extend(defs(&text, parsed.labels, from).into_iter().filter(|(at, _)| *at < cut_at));
                self.defs.sort();
                let labels = counts(&self.defs);
                let lost = self.labels.keys().any(|label| !labels.contains_key(label));
                if lost {
                    // A parse keeps only a label's first definition, so one
                    // gone may hide another outside the span: read it all.
                    self.built = false;
                    return self.sync(len, read);
                }
                let gained = labels.keys().any(|label| !self.labels.contains_key(label));
                self.labels = labels;
                return Some(if gained { 0..len } else { from..cut_at });
            }
            // Not found again: look twice as far.
            k = (k + (k - after).max(1)).min(self.starts.len());
        }
    }

    /// The blocks holding BEG..END: from the start of the block BEG is in
    /// to the start of the first block at or after END, or the end.
    pub(crate) fn blocks(&self, beg: usize, end: usize) -> Range<usize> {
        let lo = self.starts.partition_point(|&s| s <= beg);
        let from = if lo == 0 { 0 } else { self.starts[lo - 1] };
        let hi = self.starts.partition_point(|&s| s < end.max(beg + 1));
        from..self.starts.get(hi).copied().unwrap_or(self.len)
    }

    /// The top-level block starts.
    #[cfg(test)]
    pub(crate) fn starts(&self) -> &[usize] {
        &self.starts
    }

    /// The link reference definitions.
    #[cfg(test)]
    pub(crate) fn defs(&self) -> &[(usize, String)] {
        &self.defs
    }
}

/// LABELS, (byte offset into TEXT, label) ascending, with their offsets as
/// characters from BASE.
fn defs(text: &str, labels: Vec<(usize, String)>, base: usize) -> Vec<(usize, String)> {
    let offsets: Vec<usize> = labels.iter().map(|(at, _)| *at).collect();
    char_starts(text, &offsets, base).into_iter().zip(labels).map(|(at, (_, label))| (at, label)).collect()
}

/// How many of DEFS define each label.
fn counts(defs: &[(usize, String)]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for (_, label) in defs {
        *counts.entry(label.clone()).or_insert(0) += 1;
    }
    counts
}

/// TOPS, byte offsets into TEXT, as character offsets from BASE.
fn char_starts(text: &str, tops: &[usize], base: usize) -> Vec<usize> {
    if text.is_ascii() {
        return tops.iter().map(|&t| base + t).collect();
    }
    let (mut byte, mut chars) = (0, 0);
    tops.iter()
        .map(|&t| {
            chars += text[byte..t].chars().count();
            byte = t;
            base + chars
        })
        .collect()
}
