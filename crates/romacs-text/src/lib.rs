//! Valid GNU-internal text in a gap buffer. All positions are logical and
//! zero-based. Callers must supply in-bounds character-aligned coordinates.
use romacs_chars::{BytePos, CharPos, char_head_p, string_char};
use std::ops::Range;
#[cfg(test)]
thread_local! {
    static SCANNED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TOPOLOGY_REFRESHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TREE_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[inline]
fn scanned_bytes(n: usize) {
    #[cfg(test)]
    SCANNED_BYTES.with(|s| s.set(s.get() + n));
    #[cfg(not(test))]
    let _ = n;
}
#[inline]
fn newline_count(bytes: &[u8]) -> usize {
    scanned_bytes(bytes.len());
    if bytes.len() < 16 {
        bytes.iter().filter(|&&b| b == b'\n').count()
    } else {
        memchr::memchr_iter(b'\n', bytes).count()
    }
}
mod chunks;
use chunks::Chunks;
mod index;
pub mod marker_tree;
mod markers;
pub use marker_tree::{MarkerSlot, MarkerTree, ProposedMarkerObject};
pub use markers::MarkerId;
use markers::Markers;
#[cfg(test)]
mod api_tests;
#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod structural_tests;
#[cfg(test)]
mod test_support;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextPos {
    pub byte: BytePos,
    pub char: CharPos,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    pub at: TextPos,
    pub del: (usize, usize),
    pub ins: (usize, usize),
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Sum {
    bytes: usize,
    chars: usize,
    nl: usize,
}
fn head_count(b: &[u8]) -> usize {
    let mut blocks = b.chunks_exact(8);
    let mut tails = 0;
    for block in &mut blocks {
        let w = u64::from_ne_bytes(block.try_into().unwrap());
        tails += ((w & !(w << 1)) & 0x8080808080808080).count_ones() as usize;
    }
    b.len()
        - tails
        - blocks
            .remainder()
            .iter()
            .filter(|&&v| !char_head_p(v))
            .count()
}
impl Sum {
    fn plus(self, b: Self) -> Self {
        Self {
            bytes: self.bytes + b.bytes,
            chars: self.chars + b.chars,
            nl: self.nl + b.nl,
        }
    }
}
#[derive(Clone, Copy, Default, Debug)]
struct Chunk {
    bytes: u32,
    chars: u32,
    nl: u32,
}
impl Chunk {
    fn wide(self) -> Sum {
        Sum {
            bytes: self.bytes as usize,
            chars: self.chars as usize,
            nl: self.nl as usize,
        }
    }
}
impl From<Sum> for Chunk {
    fn from(s: Sum) -> Self {
        debug_assert!(s.bytes <= 2 * T);
        Self {
            bytes: s.bytes as u32,
            chars: s.chars as u32,
            nl: s.nl as u32,
        }
    }
}
/// Owns bytes, incremental summaries, ticks and generational marker identifiers.
#[derive(Clone)]
pub struct Text {
    buf: Vec<u8>,
    gap: Range<usize>,
    gap_char: CharPos,
    multibyte: bool,
    total: Sum,
    chunks: Chunks,
    // Rank, prefix and summary are valid until the next generic edit.
    cached: Option<(usize, Sum, Chunk)>,
    #[cfg(test)]
    index_work: usize,
    markers: Markers,
    modiff: u64,
    chars_modiff: u64,
    epoch: u64,
}
#[cfg(test)]
const T: usize = 8;
#[cfg(not(test))]
const T: usize = 16384;
impl Text {
    pub fn new(multibyte: bool) -> Self {
        Self {
            buf: vec![],
            gap: 0..0,
            gap_char: CharPos(0),
            multibyte,
            total: Sum::default(),
            chunks: {
                let mut chunks = Chunks::default();
                chunks.insert(0, Chunk::default());
                chunks
            },
            #[cfg(test)]
            index_work: 0,
            cached: None,
            markers: Markers::default(),
            modiff: 0,
            chars_modiff: 0,
            epoch: 0,
        }
    }
    pub fn from_bytes(b: &[u8], multibyte: bool) -> Self {
        let mut s = Self::new(multibyte);
        s.insert(BytePos(0), b, false);
        s
    }
    #[inline]
    pub fn is_multibyte(&self) -> bool {
        self.multibyte
    }
    /// Move the physical gap to a character boundary without changing content or ticks.
    pub fn move_gap_to(&mut self, at: BytePos) {
        assert!(self.is_char_boundary(at));
        let chars = self.char_of_byte(at);
        self.move_gap(at.0);
        self.gap_char = chars;
    }

    /// Reinterpret existing bytes, rebuilding summaries but never duplicating storage.
    /// Character positions of markers are recomputed from their byte positions.
    pub fn set_multibyte(&mut self, multibyte: bool) {
        if self.multibyte == multibyte {
            return;
        }
        self.multibyte = multibyte;
        let gap = self.floor_char_boundary(BytePos(self.gap.start));
        self.move_gap(gap.0);
        self.cached = None;
        let pieces = self.pieces(0, self.len_bytes());
        self.total = pieces
            .iter()
            .fold(Sum::default(), |sum, c| sum.plus(c.wide()));
        self.chunks = Chunks::from_pieces(pieces);
        if self.chunks.len() == 0 {
            self.chunks.insert(0, Chunk::default());
        }
        self.gap_char = CharPos(self.range_sum(0, self.gap.start).chars);
        let mut markers = std::mem::take(&mut self.markers);
        markers.reindex(self);
        self.markers = markers;
        self.epoch = self.epoch.checked_add(1).expect("text epoch exhausted");
    }

    /// Logical zero-based byte and character positions of the gap.
    #[inline]
    pub fn gap_start(&self) -> TextPos {
        TextPos {
            byte: BytePos(self.gap.start),
            char: self.gap_char,
        }
    }
    /// Unused capacity at the gap, in bytes.
    #[inline]
    pub fn gap_len(&self) -> usize {
        self.gap.len()
    }
    #[inline]
    pub fn len_bytes(&self) -> usize {
        self.total.bytes
    }
    #[inline]
    pub fn len_chars(&self) -> usize {
        self.total.chars
    }
    #[inline]
    pub fn len_newlines(&self) -> usize {
        self.total.nl
    }
    pub fn modiff(&self) -> u64 {
        self.modiff
    }
    /// Content changes only; restoring modification ticks cannot rewind it.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn chars_modiff(&self) -> u64 {
        self.chars_modiff
    }
    /// Restore caller-owned modification state without changing text or markers.
    pub fn set_modification_ticks(&mut self, modiff: u64, chars_modiff: u64) {
        self.modiff = modiff;
        self.chars_modiff = chars_modiff;
    }
    pub fn bump_modiff(&mut self, len: usize) {
        self.modiff = self
            .modiff
            .wrapping_add(if len == 0 { 1 } else { len.ilog2() as u64 + 1 });
    }
    /// Invalidate character-dependent observers when text is swapped without an edit.
    pub fn touch_chars(&mut self) {
        self.bump_modiff(1);
        self.chars_modiff = self.modiff;
    }
    fn changed(&mut self, n: usize) {
        if n > 0 {
            self.epoch = self.epoch.checked_add(1).expect("text epoch exhausted");
            self.bump_modiff(n);
            self.chars_modiff = self.modiff;
        }
    }
    fn sum(&self, b: &[u8]) -> Sum {
        scanned_bytes(b.len());
        if b.len() < 16 {
            let mut chars = 0;
            let mut nl = 0;
            for &byte in b {
                chars += usize::from(!self.multibyte || char_head_p(byte));
                nl += usize::from(byte == b'\n');
            }
            return Sum {
                bytes: b.len(),
                chars,
                nl,
            };
        }
        Sum {
            bytes: b.len(),
            chars: if self.multibyte && !b.is_ascii() {
                head_count(b)
            } else {
                b.len()
            },
            nl: memchr::memchr_iter(b'\n', b).count(),
        }
    }
    fn range_sum(&self, f: usize, z: usize) -> Sum {
        let (a, b) = self.segments(BytePos(f), BytePos(z));
        self.sum(a).plus(self.sum(b))
    }
    fn validate(&self, b: &[u8]) {
        if cfg!(debug_assertions) && self.multibyte {
            let mut i = 0;
            while i < b.len() {
                let n = romacs_chars::multibyte_length(&b[i..], true);
                assert!(n > 0, "invalid internal text");
                i += n;
            }
        }
    }
    #[inline]
    pub fn segments(&self, from: BytePos, to: BytePos) -> (&[u8], &[u8]) {
        let (f, z) = (from.0, to.0);
        assert!(f <= z && z <= self.len_bytes());
        let g = self.gap.len();
        if z <= self.gap.start {
            (&self.buf[f..z], &[])
        } else if f >= self.gap.start {
            (&self.buf[f + g..z + g], &[])
        } else {
            (&self.buf[f..self.gap.start], &self.buf[self.gap.end..z + g])
        }
    }
    /// Lend the physical text segment containing a logical byte, including
    /// interior character bytes. The gap is never moved and capacity is omitted.
    #[inline]
    pub fn contiguous_window_at(&self, at: BytePos) -> Option<(BytePos, &[u8])> {
        if at.0 >= self.len_bytes() {
            None
        } else if at.0 < self.gap.start {
            Some((BytePos(0), &self.buf[..self.gap.start]))
        } else {
            Some((BytePos(self.gap.start), &self.buf[self.gap.end..]))
        }
    }
    pub fn copy_to(&self, from: BytePos, to: BytePos, out: &mut Vec<u8>) {
        let (a, b) = self.segments(from, to);
        out.extend_from_slice(a);
        out.extend_from_slice(b);
    }
    pub fn byte_at(&self, at: BytePos) -> u8 {
        assert!(at.0 < self.len_bytes());
        self.buf[at.0
            + if at.0 >= self.gap.start {
                self.gap.len()
            } else {
                0
            }]
    }
    pub fn is_char_boundary(&self, at: BytePos) -> bool {
        at.0 <= self.len_bytes()
            && (!self.multibyte || at.0 == self.len_bytes() || char_head_p(self.byte_at(at)))
    }
    pub fn floor_char_boundary(&self, mut at: BytePos) -> BytePos {
        assert!(at.0 <= self.len_bytes());
        while !self.is_char_boundary(at) {
            at.0 -= 1;
        }
        at
    }
    pub fn char_at(&self, at: BytePos) -> (u32, usize) {
        debug_assert!(self.is_char_boundary(at));
        if !self.multibyte {
            (self.byte_at(at) as u32, 1)
        } else {
            let (a, _) = self.segments(at, BytePos(self.len_bytes()));
            string_char(a)
        }
    }
    fn move_gap(&mut self, p: usize) {
        if p < self.gap.start {
            self.buf
                .copy_within(p..self.gap.start, self.gap.end - (self.gap.start - p));
        } else {
            self.buf.copy_within(
                self.gap.end..self.gap.end + p - self.gap.start,
                self.gap.start,
            );
        }
        let n = self.gap.len();
        self.gap = p..p + n;
    }
    #[inline]
    fn reserve_gap(&mut self, n: usize) {
        if self.gap.len() < n {
            self.grow_gap(n);
        }
    }
    #[cold]
    #[inline(never)]
    fn grow_gap(&mut self, n: usize) {
        let size = n.max(self.len_bytes() / 64) + 2000;
        let extra = size - self.gap.len();
        let end = self.buf.len();
        self.buf.resize(end + extra, 0);
        self.buf
            .copy_within(self.gap.end..end, self.gap.end + extra);
        self.gap.end += extra;
    }
    fn edit(
        &mut self,
        from_pos: TextPos,
        to_pos: TextPos,
        b: &[u8],
        ins: Sum,
        bm: bool,
        replace: bool,
    ) -> Edit {
        let (from, to) = (from_pos.byte, to_pos.byte);
        let (fc, tc) = (from_pos.char, to_pos.char);
        assert!(from <= to && to.0 <= self.len_bytes());
        debug_assert_eq!(self.char_of_byte(from), fc);
        debug_assert_eq!(self.char_of_byte(to), tc);
        self.validate(b);
        let cached = self.cached.take();
        let del = (to - from, tc - fc);
        let result = Edit {
            at: TextPos {
                byte: from,
                char: fc,
            },
            del,
            ins: (ins.bytes, ins.chars),
        };
        if del.0 == 0 && b.is_empty() {
            return result;
        }
        let (i, s, j, e, old) = if let Some((i, prefix, old)) = cached
            && from.0 >= prefix.bytes
            && to.0 <= prefix.bytes + old.bytes as usize
        {
            (i, prefix, i, prefix, old)
        } else {
            let (i, s) = self.locate(from.0, |s| s.bytes);
            let (j, e) = if from == to {
                (i, s)
            } else {
                self.locate(to.0, |s| s.bytes)
            };
            (i, s, j, e, self.chunks[i])
        };
        let sum = if i == j {
            let old = old.wide();
            let (a, b) = self.segments(from, to);
            let removed = Sum {
                bytes: del.0,
                chars: del.1,
                nl: newline_count(a) + newline_count(b),
            };
            Sum {
                bytes: old.bytes - removed.bytes + ins.bytes,
                chars: old.chars - removed.chars + ins.chars,
                nl: old.nl - removed.nl + ins.nl,
            }
        } else {
            self.range_sum(s.bytes, from.0)
                .plus(ins)
                .plus(self.range_sum(to.0, e.bytes + self.chunks[j].bytes as usize))
        };
        let old_nl = if i == j {
            old.nl as usize
        } else {
            e.nl + self.chunks[j].nl as usize - s.nl
        };
        if to.0 == self.gap.start {
            // Deleting immediately before the gap only exposes more gap;
            // moving those doomed bytes to its right would copy then discard.
            self.gap.start = from.0;
        } else {
            self.move_gap(from.0);
            self.gap.end += del.0;
        }
        self.reserve_gap(b.len());
        self.buf[self.gap.start..self.gap.start + b.len()].copy_from_slice(b);
        self.gap.start += b.len();
        self.gap_char = CharPos(fc.0 + ins.chars);
        self.total = Sum {
            bytes: self.total.bytes - del.0 + ins.bytes,
            chars: self.total.chars - del.1 + ins.chars,
            nl: self.total.nl - old_nl + sum.nl,
        };
        if i == j && sum.bytes >= T / 4 && sum.bytes <= 2 * T {
            let chunk = sum.into();
            self.chunks.replace_summary(i, old.wide(), sum);
            self.cached = Some((i, s, chunk));
            #[cfg(test)]
            {
                self.index_work += std::mem::take(&mut self.chunks.work);
            }
            self.markers.adjust(result, bm, replace);
            self.changed(del.1 + ins.chars);
            return result;
        }
        if i == j && sum.bytes <= 2 * T {
            self.chunks.set(i, sum.into());
        } else {
            let sums = if sum.bytes > 2 * T {
                self.pieces(s.bytes, s.bytes + sum.bytes)
            } else {
                vec![sum.into()]
            };
            if i == 0 && j + 1 == self.chunks.len() {
                self.chunks = Chunks::from_pieces(sums);
            } else {
                for _ in i..=j {
                    self.chunks.remove(i);
                }
                for (offset, chunk) in sums.into_iter().enumerate() {
                    self.chunks.insert(i + offset, chunk);
                }
            }
        }
        let mut k = i.saturating_sub(1);
        while k + 1 < self.chunks.len() && k <= i + 1 {
            if (self.chunks[k].bytes < (T / 4) as u32 || self.chunks[k + 1].bytes < (T / 4) as u32)
                && self.chunks[k].bytes + self.chunks[k + 1].bytes <= (2 * T) as u32
            {
                let right = self.chunks.remove(k + 1).wide();
                self.chunks.set(k, self.chunks[k].wide().plus(right).into());
            } else {
                k += 1;
            }
        }
        #[cfg(test)]
        {
            self.index_work += std::mem::take(&mut self.chunks.work);
        }
        self.markers.adjust(result, bm, replace);
        self.changed(del.1 + ins.chars);
        result
    }
    pub fn insert(&mut self, at: BytePos, bytes: &[u8], before_markers: bool) -> Edit {
        let pos = TextPos {
            byte: at,
            char: self.char_of_byte(at),
        };
        let chars = if self.multibyte {
            head_count(bytes)
        } else {
            bytes.len()
        };
        self.insert_measured(pos, bytes, chars, before_markers)
    }
    /// Insert with coordinates and character length already measured by the caller.
    pub fn insert_measured(
        &mut self,
        position: TextPos,
        bytes: &[u8],
        chars: usize,
        before_markers: bool,
    ) -> Edit {
        let at = position.byte;
        let fc = position.char;
        assert!(at.0 <= self.len_bytes());
        debug_assert_eq!(self.char_of_byte(at), fc);
        self.validate(bytes);
        debug_assert_eq!(
            chars,
            if self.multibyte {
                head_count(bytes)
            } else {
                bytes.len()
            }
        );
        if self.len_bytes() == 0 && bytes.len() > 2 * T {
            return self.load_empty(bytes, chars, before_markers);
        }
        let ins = Sum {
            bytes: bytes.len(),
            chars,
            nl: newline_count(bytes),
        };
        let edit = Edit {
            at: TextPos { byte: at, char: fc },
            del: (0, 0),
            ins: (ins.bytes, ins.chars),
        };
        if bytes.is_empty() {
            return edit;
        }
        let cached = self.cached.filter(|&(_, prefix, old)| {
            at.0 >= prefix.bytes && at.0 <= prefix.bytes + old.bytes as usize
        });
        let destination = if let Some((i, prefix, old)) = cached {
            if old.bytes as usize + ins.bytes <= 2 * T {
                self.chunks.add(i, ins);
                Some((i, prefix, old.wide().plus(ins).into()))
            } else {
                self.split_cached_insert(at, ins, i, prefix, old)
            }
        } else {
            self.chunks
                .insert_summary(at.0, ins)
                .map(|(i, prefix, old)| (i, prefix, old.wide().plus(ins).into()))
        };
        if let Some(destination) = destination {
            // The tree already contains the inserted summary, including any
            // local split. Publish bytes and the final cached chunk together.
            if self.gap.start != at.0 {
                self.move_gap(at.0);
            }
            self.reserve_gap(bytes.len());
            self.buf[self.gap.start..self.gap.start + bytes.len()].copy_from_slice(bytes);
            self.gap.start += bytes.len();
            self.gap_char = CharPos(fc.0 + ins.chars);
            self.total = self.total.plus(ins);
            self.cached = Some(destination);
            #[cfg(test)]
            {
                self.index_work += std::mem::take(&mut self.chunks.work);
            }
            self.markers.adjust(edit, before_markers, false);
            self.changed(ins.chars);
            return edit;
        }
        let edit = self.edit(position, position, bytes, ins, before_markers, false);
        let (i, prefix) = self.locate(self.gap.start, |s| s.bytes);
        self.cached = Some((i, prefix, self.chunks[i]));
        edit
    }
    fn split_cached_insert(
        &mut self,
        at: BytePos,
        ins: Sum,
        i: usize,
        prefix: Sum,
        old: Chunk,
    ) -> Option<(usize, Sum, Chunk)> {
        let left_bytes = at.0 - prefix.bytes;
        let right_bytes = old.bytes as usize - left_bytes + ins.bytes;
        // Keep interior chunks substantial, but let the last chunk start small
        // and grow. Every split still updates only logarithmic AVL paths.
        if left_bytes < T / 4
            || right_bytes > 2 * T
            || (i + 1 != self.chunks.len() && right_bytes < T / 4)
        {
            return None;
        }
        // Only the old suffix needs measuring: appends scan no old bytes and
        // streaming before a prompt scans only that prompt, not the full chunk.
        let suffix = self.range_sum(at.0, prefix.bytes + old.bytes as usize);
        let left = Sum {
            bytes: left_bytes,
            chars: old.chars as usize - suffix.chars,
            nl: old.nl as usize - suffix.nl,
        };
        let right = ins.plus(suffix);
        self.chunks.replace_summary(i, old.wide(), left);
        self.chunks.insert(i + 1, right.into());
        Some((i + 1, prefix.plus(left), right.into()))
    }
    pub fn delete(&mut self, from: BytePos, to: BytePos) -> Edit {
        let from = TextPos {
            byte: from,
            char: self.char_of_byte(from),
        };
        let to = TextPos {
            byte: to,
            char: self.char_of_byte(to),
        };
        self.delete_measured(from, to)
    }
    pub fn delete_measured(&mut self, from: TextPos, to: TextPos) -> Edit {
        self.edit(from, to, &[], Sum::default(), false, false)
    }
    pub fn replace(&mut self, from: BytePos, to: BytePos, bytes: &[u8]) -> Edit {
        let from = TextPos {
            byte: from,
            char: self.char_of_byte(from),
        };
        let to = TextPos {
            byte: to,
            char: self.char_of_byte(to),
        };
        let chars = if self.multibyte {
            head_count(bytes)
        } else {
            bytes.len()
        };
        self.replace_measured(from, to, bytes, chars)
    }
    pub fn replace_measured(
        &mut self,
        from: TextPos,
        to: TextPos,
        bytes: &[u8],
        chars: usize,
    ) -> Edit {
        debug_assert_eq!(
            chars,
            if self.multibyte {
                head_count(bytes)
            } else {
                bytes.len()
            }
        );
        let ins = Sum {
            bytes: bytes.len(),
            chars,
            nl: newline_count(bytes),
        };
        self.edit(from, to, bytes, ins, false, true)
    }
    fn load_empty(&mut self, bytes: &[u8], chars: usize, before_markers: bool) -> Edit {
        self.move_gap(0);
        self.reserve_gap(bytes.len());
        self.buf[..bytes.len()].copy_from_slice(bytes);
        self.gap.start = bytes.len();
        self.gap_char = CharPos(chars);
        self.total = Sum {
            bytes: bytes.len(),
            chars,
            nl: 0,
        };
        let chunks: Vec<Chunk> = if chars == bytes.len() {
            bytes
                .chunks(T)
                .map(|b| {
                    Sum {
                        bytes: b.len(),
                        chars: b.len(),
                        nl: newline_count(b),
                    }
                    .into()
                })
                .collect()
        } else {
            self.pieces(0, bytes.len())
        };
        self.total.nl = chunks.iter().map(|c| c.nl as usize).sum();
        self.chunks = Chunks::from_pieces(chunks);
        let last = self.chunks.len() - 1;
        let chunk = self.chunks[last];
        self.cached = Some((
            last,
            Sum {
                bytes: self.total.bytes - chunk.bytes as usize,
                chars: self.total.chars - chunk.chars as usize,
                nl: self.total.nl - chunk.nl as usize,
            },
            chunk,
        ));
        #[cfg(test)]
        {
            self.index_work += std::mem::take(&mut self.chunks.work);
        }
        let result = Edit {
            at: TextPos {
                byte: BytePos(0),
                char: CharPos(0),
            },
            del: (0, 0),
            ins: (bytes.len(), chars),
        };
        self.markers.adjust(result, before_markers, false);
        self.changed(chars);
        result
    }
    pub fn marker_new(&mut self, at: BytePos, after: bool) -> MarkerId {
        let c = self.char_of_byte(at);
        self.markers.new_marker(at.0, c.0, after)
    }
    pub fn marker_set(&mut self, m: MarkerId, at: BytePos) -> bool {
        let c = self.char_of_byte(at);
        self.markers.set(m, at.0, c.0)
    }
    pub fn marker_set_type(&mut self, m: MarkerId, after: bool) -> bool {
        self.markers.set_type(m, after)
    }
    pub fn marker_pos(&self, m: MarkerId) -> Option<TextPos> {
        self.markers.pos(m)
    }
    pub fn marker_free(&mut self, m: MarkerId) -> bool {
        self.markers.free(m)
    }
    pub fn marker_count(&self) -> usize {
        self.markers.len()
    }
    #[doc(hidden)]
    pub fn check_invariants(&self) {
        assert_eq!(self.buf.len() - self.gap.len(), self.len_bytes());
        assert!(self.is_char_boundary(BytePos(self.gap.start)));
        assert_eq!(self.gap_char.0, self.range_sum(0, self.gap.start).chars);
        let mut sum = Sum::default();
        for i in 0..self.chunks.len() {
            let c = self.chunks[i].wide();
            assert!(c.bytes <= 2 * T);
            assert!(c.bytes > 0 || self.chunks.len() == 1);
            assert_eq!(c, self.range_sum(sum.bytes, sum.bytes + c.bytes));
            sum = sum.plus(c);
            assert!(self.is_char_boundary(BytePos(sum.bytes)));
        }
        assert_eq!(sum, self.total);
        self.chunks.check();
        assert_eq!(self.chunks.total(), self.total);
        self.markers.check(self);
    }
}
