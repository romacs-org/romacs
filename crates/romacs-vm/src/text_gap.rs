//! The search view of a buffer's text: GNU's own layout, the text in
//! internal form around one gap, kept up to date edit by edit instead of
//! flattened again from the rope after each one. A search asks for a
//! window, a contiguous run of the text holding everything it can read;
//! the gap moves out of the way only when it lies inside that window, so a
//! loop of searches and edits moves it about as far as the edits are
//! apart, not the length of the buffer, each time.
use std::ops::Range;

/// The text, in GNU's internal form, with a gap at `gap`.
#[derive(Clone, Default)]
pub(crate) struct GapText {
    /// The text before the gap, the gap, then the text after it.
    buf: Vec<u8>,
    /// Where the gap is, as a position of the text.
    gap: usize,
    gap_len: usize,
    /// Bytes edits moved since the last window: past the text's length,
    /// keeping the view up to date costs more than building it again.
    moved: usize,
}
romacs_snapshot::snapshot_struct!(GapText { buf, gap, gap_len, moved });

/// A body's search view, while it pays: absent until a search after an
/// edit, dropped when keeping it costs more than a rebuild. A copy of the
/// body starts without one, as a cache does.
#[derive(Default)]
pub(crate) struct SearchView(pub(crate) Option<GapText>);
romacs_snapshot::snapshot_tuple!(SearchView(view));
impl Clone for SearchView {
    fn clone(&self) -> Self {
        SearchView(None)
    }
}
impl SearchView {
    /// R replaced by INSERTED, or the view dropped.
    pub(crate) fn replace<'a>(&mut self, r: Range<usize>, inserted: impl FnOnce() -> std::borrow::Cow<'a, [u8]>) {
        if let Some(view) = &mut self.0 && !view.replace(r, &inserted()) {
            self.0 = None;
        }
    }
    pub(crate) fn storage_bytes(&self, capacity: bool) -> usize {
        self.0.as_ref().map_or(0, |v| if capacity { v.buf.capacity() } else { v.len() })
    }
}

/// The gap a growing view leaves free, beyond what the insertion needs.
const GAP_SLACK: usize = 4096;

/// Internal multibyte text starts each character with a byte that is not a
/// continuation byte (10xxxxxx).
fn continues_character(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

impl GapText {
    /// The text CHUNKS hold, LEN bytes in all, its gap at the end; RAW
    /// lists the raw-byte placeholders to lower to GNU's internal form.
    pub(crate) fn new<'a>(chunks: impl Iterator<Item = &'a [u8]>, len: usize, raw: &[u32]) -> Self {
        super::count_copied(len);
        let mut buf = Vec::with_capacity(len + GAP_SLACK);
        chunks.for_each(|c| buf.extend_from_slice(c));
        debug_assert_eq!(buf.len(), len);
        for &p in raw {
            buf[p as usize] -= super::RAW_LEAD_SHIFT;
        }
        buf.resize(len + GAP_SLACK, 0);
        GapText { buf, gap: len, gap_len: GAP_SLACK, moved: 0 }
    }

    pub(crate) fn len(&self) -> usize {
        self.buf.len() - self.gap_len
    }

    /// The byte at text position AT (`AT < len`).
    fn byte(&self, at: usize) -> u8 {
        self.buf[if at < self.gap { at } else { at + self.gap_len }]
    }

    /// Move the gap to text position TO; returns the bytes moved.
    fn move_gap(&mut self, to: usize) -> usize {
        let (gap, len) = (self.gap, self.gap_len);
        if to < gap {
            self.buf.copy_within(to..gap, to + len);
        } else if to > gap {
            self.buf.copy_within(gap + len..to + len, gap);
        }
        self.gap = to;
        super::count_copied(to.abs_diff(gap));
        to.abs_diff(gap)
    }

    /// The text's R replaced by INSERTED. False when edits have moved more
    /// since the last window than the text holds: the caller drops the view.
    pub(crate) fn replace(&mut self, r: Range<usize>, inserted: &[u8]) -> bool {
        debug_assert!(r.start <= r.end && r.end <= self.len());
        // Delete by widening the gap over R from its nearer end.
        let to = if self.gap >= r.end { r.end } else { r.start };
        self.moved += self.move_gap(to);
        self.gap = r.start;
        self.gap_len += r.end - r.start;
        if inserted.len() > self.gap_len {
            self.grow(inserted.len());
        }
        self.buf[self.gap..self.gap + inserted.len()].copy_from_slice(inserted);
        self.gap += inserted.len();
        self.gap_len -= inserted.len();
        self.moved <= self.len() + GAP_SLACK
    }

    /// A gap of at least NEED bytes, and slack beyond them.
    fn grow(&mut self, need: usize) {
        let extra = need + GAP_SLACK + self.len() / 8 - self.gap_len;
        let after = self.gap + self.gap_len;
        super::count_copied(self.len());
        self.buf.splice(after..after, std::iter::repeat_n(0, extra));
        self.gap_len += extra;
    }

    /// A contiguous run of the text that holds NEED and lies within
    /// BOUNDS, both text ranges, for [`Self::slice`]. It extends to a
    /// bound of BOUNDS on the side away from the gap, and to the gap (or
    /// the other bound) on the gap's side, so a run that does not end at a
    /// bound ends at least at NEED's end. The gap moves only out of NEED,
    /// to a character boundary, and as little as it can.
    pub(crate) fn window(&mut self, need: Range<usize>, bounds: Range<usize>) -> Range<usize> {
        debug_assert!(bounds.start <= need.start && need.start <= need.end && need.end <= bounds.end && bounds.end <= self.len());
        self.moved = 0;
        if self.gap > need.start && self.gap < need.end {
            let to = if self.gap - need.start <= need.end - self.gap {
                let mut at = need.start;
                while at > 0 && continues_character(self.byte(at)) { at -= 1; }
                at
            } else {
                let mut at = need.end;
                while at < self.len() && continues_character(self.byte(at)) { at += 1; }
                at
            };
            self.move_gap(to);
        }
        if self.gap <= need.start {
            self.gap.max(bounds.start)..bounds.end
        } else {
            bounds.start..self.gap.min(bounds.end)
        }
    }

    /// The bytes of R, a run that does not hold the gap.
    pub(crate) fn slice(&self, r: Range<usize>) -> &[u8] {
        assert!(r.end <= self.gap || r.start >= self.gap, "search window across the gap");
        let shift = if r.start >= self.gap { self.gap_len } else { 0 };
        &self.buf[r.start + shift..r.end + shift]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(g: &GapText) -> Vec<u8> {
        let mut out = g.buf[..g.gap].to_vec();
        out.extend_from_slice(&g.buf[g.gap + g.gap_len..]);
        out
    }

    /// A deterministic sequence for edits.
    fn lcg(seed: &mut u64) -> usize {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 33) as usize
    }

    #[test]
    fn edits_and_windows_match_a_flat_oracle() {
        let mut oracle = b"hello world, \xc3\xa9t\xc3\xa9 and more text\n".repeat(50);
        let mut g = GapText::new(oracle.chunks(7), oracle.len(), &[]);
        let mut seed = 7;
        for _ in 0..2000 {
            let len = oracle.len();
            let a = lcg(&mut seed) % (len + 1);
            let b = (a + lcg(&mut seed) % 9).min(len);
            let inserted = &b"xy\xc3\xa9z"[..lcg(&mut seed) % 6 / 2 * 2];
            oracle.splice(a..b, inserted.iter().copied());
            g.replace(a..b, inserted);
            g.moved = 0;
            assert_eq!(text(&g), oracle);
            let len = oracle.len();
            let s = lcg(&mut seed) % (len + 1);
            let e = (s + lcg(&mut seed) % 40).min(len);
            let bounds = s.saturating_sub(lcg(&mut seed) % 20)..(e + lcg(&mut seed) % 20).min(len);
            let window = g.window(s..e, bounds.clone());
            assert!(window.start <= s && window.end >= e);
            assert!(window.start == bounds.start || window.end == bounds.end);
            assert_eq!(g.slice(window.clone()), &oracle[window]);
        }
    }

    #[test]
    fn a_window_beside_the_gap_moves_nothing() {
        let mut g = GapText::new([b"abcdefghij".as_slice()].into_iter(), 10, &[]);
        g.replace(4..5, b"E");
        assert_eq!(g.gap, 5);
        let before = g.buf.clone();
        let window = g.window(6..8, 0..10);
        assert_eq!((window.clone(), g.slice(window)), (5..10, b"fghij".as_slice()));
        assert_eq!(g.buf, before);
        let window = g.window(1..3, 0..10);
        assert_eq!((window.clone(), g.slice(window)), (0..5, b"abcdE".as_slice()));
    }

    #[test]
    fn moving_more_than_the_text_asks_for_a_rebuild() {
        let mut g = GapText::new([vec![b'a'; 100].as_slice()].into_iter(), 100, &[]);
        let mut kept = true;
        for i in 0..200 {
            kept &= g.replace(if i % 2 == 0 { 0..0 } else { 99..99 }, b"");
        }
        assert!(!kept);
    }
}
