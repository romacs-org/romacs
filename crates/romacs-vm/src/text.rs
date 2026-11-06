//! UTF-8 rope storage; properties and restrictions remain byte-canonical.
//! The cached ASCII property changes conversion cost, never the payload type.
use crate::properties::Properties;
use crate::{Result, Value, coordinates::*, indirect::Shared};
use std::borrow::Cow;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
#[path = "text_gap.rs"]
mod gap;

#[cfg(test)]
thread_local! {
    /// Bytes this thread's texts copied whole or moved: flattenings,
    /// search views built and their gap moves (the scaling probes).
    static COPIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// Count N bytes copied (test builds only).
pub(crate) fn count_copied(n: usize) {
    #[cfg(test)]
    COPIED.with(|c| c.set(c.get() + n));
    #[cfg(not(test))]
    let _ = n;
}
/// The bytes this thread's texts have copied so far.
#[cfg(test)]
pub(crate) fn copied_bytes() -> usize {
    COPIED.with(|c| c.get())
}

#[derive(Clone, Default)]
struct ChunkCursor {
    text: String,
    byte: usize,
    scalar: usize,
    chars: usize,
    middle_byte: usize,
    middle_char: usize,
}
romacs_snapshot::snapshot_struct!(ChunkCursor { text, byte, scalar, chars, middle_byte, middle_char });
impl ChunkCursor {
    fn fill(&mut self, text: &str, byte: usize, scalar: usize) {
        self.text.clear();
        self.text.push_str(text);
        self.byte = byte;
        self.scalar = scalar;
        self.chars = text.chars().count();
        self.middle_byte = text.len() / 2;
        while !text.is_char_boundary(self.middle_byte) {
            self.middle_byte += 1;
        }
        self.middle_char = text[..self.middle_byte].chars().count();
    }
    fn byte_of_char(&self, local: usize) -> (usize, usize) {
        if self.text.len() == self.chars {
            return (local, 0);
        }
        let (byte, scalar) = if local >= self.middle_char {
            (self.middle_byte, self.middle_char)
        } else {
            (0, 0)
        };
        let at = str_indices::chars::to_byte_idx(&self.text[byte..], local - scalar);
        (byte + at, at)
    }
}
/// The text of a buffer and its indirect buffers (GNU's `struct
/// buffer_text`): the characters, their properties and the caches that
/// read them. Each buffer's view of it is a [`Text`].
#[derive(Clone)]
pub struct TextBody {
    rope: ropey_stable::Rope,
    bytes_len: usize,
    chars_len: usize,
    tail: String,
    tail_chars: usize,
    flat: std::sync::OnceLock<String>,
    /// The byte offsets of the placeholders standing for raw bytes
    /// (`internal_to_rope`), sorted; empty for pure Unicode text.
    raw: Vec<u32>,
    /// `bytes()` in GNU's internal form, built once `raw` is non-empty.
    flat_internal: std::sync::OnceLock<Vec<u8>>,
    /// The text kept contiguous for searches across edits (text_gap.rs).
    search: gap::SearchView,
    is_ascii: bool,
    coordinate: std::cell::Cell<(usize, usize)>,
    chunk: std::cell::RefCell<ChunkCursor>,
    cold_chunks: std::cell::RefCell<std::collections::BTreeMap<usize, ChunkCursor>>,
    cold_order: std::cell::RefCell<std::collections::VecDeque<usize>>,
    #[cfg(test)]
    scanned_bytes: std::cell::Cell<usize>,
    pub(crate) properties: Properties,
    #[cfg(test)]
    coordinate_steps: std::cell::Cell<usize>,
    #[cfg(test)]
    property_edit_visits: usize,
    /// The tree-sitter parsers of this text (GNU's base buffer
    /// `ts_parser_list'), whose trees every change edits.
    pub(crate) treesit: crate::treesit::Registry,
}
romacs_snapshot::snapshot_struct!(TextBody { rope, bytes_len, chars_len, tail, tail_chars, flat, raw, flat_internal, search, is_ascii, coordinate, chunk, cold_chunks, cold_order, #[cfg(test)] scanned_bytes, properties, #[cfg(test)] coordinate_steps, #[cfg(test)] property_edit_visits, treesit });
// GNU's eight-bit characters (#x3FFF80..#x3FFFFF, the raw bytes 0x80..0xFF)
// cannot be UTF-8, which the rope requires. Raw byte B is stored as the
// placeholder character U+00B, whose UTF-8 is GNU's internal form of the
// raw byte with the lead byte RAW_LEAD_SHIFT higher: the same width, so
// every byte position agrees with GNU; `TextBody::raw` lists the
// placeholders that stand for raw bytes. Every write converts through
// `internal_to_rope`, every Lisp-visible read back through
// `rope_to_internal`.
pub(crate) const RAW_LEAD_SHIFT: u8 = 2;

/// Newline scans of text read where it lies, a slice or a buffer's rope:
/// a newline byte is never part of another character.
pub(crate) trait Newlines {
    /// The first newline in FROM..TO.
    fn next_newline(&self, from: usize, to: usize) -> Option<usize>;
    /// The last newline in TO..FROM.
    fn previous_newline(&self, from: usize, to: usize) -> Option<usize>;
}
impl Newlines for [u8] {
    fn next_newline(&self, from: usize, to: usize) -> Option<usize> {
        self[from..to].iter().position(|&c| c == b'\n').map(|i| from + i)
    }
    fn previous_newline(&self, from: usize, to: usize) -> Option<usize> {
        self[to..from].iter().rposition(|&c| c == b'\n').map(|i| to + i)
    }
}
impl Newlines for Vec<u8> {
    fn next_newline(&self, from: usize, to: usize) -> Option<usize> {
        self.as_slice().next_newline(from, to)
    }
    fn previous_newline(&self, from: usize, to: usize) -> Option<usize> {
        self.as_slice().previous_newline(from, to)
    }
}
impl Newlines for TextBody {
    fn next_newline(&self, from: usize, to: usize) -> Option<usize> {
        self.newline_after(from, to)
    }
    fn previous_newline(&self, from: usize, to: usize) -> Option<usize> {
        self.newline_before(from, to)
    }
}

/// Rope text and the local offsets of its raw bytes for GNU-internal
/// BYTES; borrowed when they are plain UTF-8 (no scan beyond validation).
fn internal_to_rope(bytes: &[u8]) -> Result<(Cow<'_, str>, Vec<u32>)> {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok((Cow::Borrowed(s), Vec::new()));
    }
    let mut text = Vec::with_capacity(bytes.len());
    let mut raw = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let (code, width) = crate::lstring::decode_char(rest).ok_or("invalid Unicode text")?;
        if code > crate::lstring::MAX_5_BYTE_CHAR {
            raw.push(text.len() as u32);
            text.push(rest[0] + RAW_LEAD_SHIFT);
            text.push(rest[1]);
        } else if char::from_u32(code).is_some() {
            text.extend_from_slice(&rest[..width]);
        } else {
            return Err("invalid Unicode text".into());
        }
        rest = &rest[width..];
    }
    let text = String::from_utf8(text).map_err(|_| "invalid Unicode text")?;
    Ok((Cow::Owned(text), raw))
}

/// The offsets of RAW inside RANGE.
fn raw_in(raw: &[u32], range: Range<usize>) -> &[u32] {
    let start = raw.partition_point(|&p| (p as usize) < range.start);
    let end = raw.partition_point(|&p| (p as usize) < range.end);
    &raw[start..end]
}

/// RANGE of the rope replaced by INSERTED bytes whose raw offsets are LOCAL.
fn raw_splice(raw: &mut Vec<u32>, range: Range<usize>, inserted: usize, local: &[u32]) {
    if raw.is_empty() && local.is_empty() {
        return;
    }
    let start = raw.partition_point(|&p| (p as usize) < range.start);
    let end = raw.partition_point(|&p| (p as usize) < range.end);
    let delta = inserted as i64 - (range.end - range.start) as i64;
    for p in &mut raw[end..] {
        *p = (i64::from(*p) + delta) as u32;
    }
    raw.splice(start..end, local.iter().map(|&p| p + range.start as u32));
}

/// Rope BYTES starting at BASE in GNU's internal form.
fn rope_to_internal(bytes: &[u8], base: usize, raw: &[u32]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for &p in raw_in(raw, base..base + bytes.len()) {
        out[p as usize - base] -= RAW_LEAD_SHIFT;
    }
    out
}

/// Rope text S, whose raw bytes are at LOCAL, in GNU's internal form.
fn internal_of<'a>(s: &'a str, local: &[u32]) -> Cow<'a, [u8]> {
    if local.is_empty() { Cow::Borrowed(s.as_bytes()) } else { Cow::Owned(rope_to_internal(s.as_bytes(), 0, local)) }
}

/// The byte a raw placeholder's UTF-8 LEAD and continuation CONT stand for.
fn raw_byte(lead: u8, cont: u8) -> u8 {
    0x80 | (lead & 1) << 6 | (cont & 0x3f)
}

/// copy_text from unibyte to multibyte (insert_from_string, raw-text
/// decoding): every byte above 127 becomes a raw-byte character.
pub(crate) fn octets_to_multibyte(octets: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(octets.len());
    for &byte in octets {
        if byte < 128 { out.push(byte); continue; }
        let mut encoded = [0; 5];
        let width = crate::lstring::char_string(crate::lstring::BYTE8_OFFSET + u32::from(byte), &mut encoded);
        out.extend_from_slice(&encoded[..width]);
    }
    out
}

/// GNU str_as_multibyte over unibyte OCTETS: valid UTF-8 sequences of
/// scalars stay, any other byte above 127 becomes a raw-byte character.
/// Returns the internal bytes and the octet indices that grew a byte.
pub(crate) fn octets_as_multibyte(octets: &[u8]) -> (Vec<u8>, Vec<usize>) {
    let mut out = Vec::with_capacity(octets.len());
    let mut grown = Vec::new();
    let mut at = 0;
    while at < octets.len() {
        let tail = &octets[at..];
        match crate::lstring::decode_char(tail) {
            Some((code, width)) if tail[0] < 0x80 || (tail[0] >= 0xc2 && char::from_u32(code).is_some()) => {
                out.extend_from_slice(&tail[..width]);
                at += width;
            }
            _ => {
                if tail[0] >= 0x80 {
                    grown.push(at);
                    let mut encoded = [0; 5];
                    let n = crate::lstring::char_string(crate::lstring::BYTE8_OFFSET + u32::from(tail[0]), &mut encoded);
                    out.extend_from_slice(&encoded[..n]);
                } else {
                    out.push(tail[0]);
                }
                at += 1;
            }
        }
    }
    (out, grown)
}

/// One buffer's view of its text: the shared [`TextBody`], and this
/// buffer's own point, narrowing, syntax and category tables and identity.
#[derive(Clone)]
pub struct Text {
    body: Shared<TextBody>,
    restriction: Range<ByteOffset>,
    pub(crate) state: crate::bytes::BufferState,
    pub(crate) point: ByteOffset,
    pub(crate) syntax_table: Value,
    /// The buffer's category table; nil means the standard one (category.c).
    pub(crate) category_table: Value,
    /// GNU's `ts_linecol_begv/point/zv' caches; tracking once BEGV's is
    /// set (treesit.c).
    pub(crate) ts_linecols: [crate::treesit::Linecol; 3],
}
romacs_snapshot::snapshot_struct!(Text { body, restriction, state, point, syntax_table, category_table, ts_linecols });
impl std::ops::Deref for Text {
    type Target = TextBody;
    fn deref(&self) -> &TextBody {
        &self.body
    }
}
impl std::ops::DerefMut for Text {
    fn deref_mut(&mut self) -> &mut TextBody {
        &mut self.body
    }
}
impl Text {
    pub fn new(bytes: &[u8]) -> Result<Self> {
        let (s, raw) = internal_to_rope(bytes)?;
        let rope = ropey_stable::Rope::from_str(&s);
        let chars_len = rope.len_chars();
        Ok(Self {
            body: Shared::new(TextBody {
                rope,
                bytes_len: s.len(),
                chars_len,
                tail: String::new(),
                tail_chars: 0,
                flat: std::sync::OnceLock::new(),
                raw,
                flat_internal: std::sync::OnceLock::new(),
                search: Default::default(),
                is_ascii: s.is_ascii(),
                coordinate: std::cell::Cell::new((0, 0)),
                chunk: std::cell::RefCell::new(ChunkCursor::default()),
                cold_chunks: std::cell::RefCell::new(std::collections::BTreeMap::new()),
                cold_order: std::cell::RefCell::new(std::collections::VecDeque::new()),
                #[cfg(test)]
                scanned_bytes: std::cell::Cell::new(0),
                properties: Properties::new(),
                #[cfg(test)]
                coordinate_steps: std::cell::Cell::new(0),
                #[cfg(test)]
                property_edit_visits: 0,
                treesit: Default::default(),
            }),
            restriction: ByteOffset(0)..ByteOffset(s.len()),
            state: Default::default(),
            point: ByteOffset(0),
            syntax_table: Value::NIL,
            category_table: Value::NIL,
            ts_linecols: [crate::treesit::EMPTY_LINECOL; 3],
        })
    }
    /// A view of this text for a new indirect buffer: the same body and
    /// modification counts, with this view's point and narrowing (GNU
    /// `make-indirect-buffer` starts at its base's) and its own identity.
    pub(crate) fn share(&self) -> Self {
        let mut state = crate::bytes::BufferState::default();
        state.modification = self.state.modification.share();
        Self {
            body: self.body.share(),
            restriction: self.restriction.clone(),
            state,
            point: self.point,
            syntax_table: Value::NIL,
            category_table: Value::NIL,
            ts_linecols: [crate::treesit::EMPTY_LINECOL; 3],
        }
    }
    /// The text this view reads.
    pub(crate) fn body(&self) -> &TextBody {
        &self.body
    }
    /// Whether THIS and OTHER are views of one body.
    #[cfg(test)]
    pub(crate) fn shares_body(&self, other: &Self) -> bool {
        self.body.same(&other.body)
    }
    /// The unibyte storage of this text (set-buffer-multibyte nil): each
    /// raw-byte character becomes its byte, so positions after one move
    /// back (`raw_compaction`); the caller moves the anchors.
    pub(crate) fn binary(&self) -> crate::bytes::ByteBuffer {
        let text = self.as_str().as_bytes();
        let mut bytes = Vec::with_capacity(text.len());
        let mut last = 0;
        for &p in &self.raw {
            let p = p as usize;
            bytes.extend_from_slice(&text[last..p]);
            bytes.push(raw_byte(text[p], text[p + 1]));
            last = p + 2;
        }
        bytes.extend_from_slice(&text[last..]);
        let map = |at: ByteOffset| ByteOffset(self.raw_compaction(at.0));
        crate::bytes::ByteBuffer {
            state: self.state.clone(),
            bytes,
            syntax_table: self.syntax_table,
            category_table: self.category_table,
            case_tables: [Value::NIL; 4],
            local_keymap: Value::NIL,
            point: map(self.point),
            restriction: map(self.restriction.start)..map(self.restriction.end),
            anchors: crate::anchors::AnchorTree::new(u32::try_from(self.len_bytes()).expect("buffer exceeds anchor coordinate limit")),
            properties: self
                .properties
                .to_map()
                .into_iter()
                .filter(|(at, _)| self.boundary(*at).is_ok())
                .map(|(at, values)| (map(at), values))
                .collect(),
            had_intervals: self.properties.has_intervals(),
            per_buffer: Default::default(),
            held: None,
            mirror: None,
        }
    }
    /// The multibyte text of unibyte B (set-buffer-multibyte t): valid
    /// UTF-8 stays, other bytes become raw-byte characters, two bytes
    /// each, so positions after one move forward. Returns the text and
    /// the octet indices that grew, for the caller's anchors.
    pub(crate) fn decode_binary(b: &crate::bytes::ByteBuffer) -> Result<(Self, Vec<usize>)> {
        let (internal, grown) = octets_as_multibyte(&b.bytes);
        let map = |at: ByteOffset| ByteOffset(at.0 + grown.partition_point(|&g| g < at.0));
        let mut text = Self::new(&internal)?;
        text.boundary(map(b.point))?;
        text.validate_range(&(map(b.restriction.start)..map(b.restriction.end)))?;
        let properties: std::collections::BTreeMap<ByteOffset, _> = b.properties.iter().map(|(&at, values)| (map(at), values.clone())).collect();
        for &p in properties.keys() {
            text.boundary(p)?;
        }
        text.point = map(b.point);
        text.state = b.state.clone();
        text.syntax_table = b.syntax_table;
        text.category_table = b.category_table;
        text.restriction = map(b.restriction.start)..map(b.restriction.end);
        text.properties = Properties::from_map(&properties);
        for (&at, values) in &properties {
            if at.0 < text.len_bytes() {
                let width = text.as_str()[at.0..].chars().next().unwrap().len_utf8();
                for (key, value) in values {
                    text.properties
                        .put(at..ByteOffset(at.0 + width), key, *value);
                }
            }
        }
        Ok((text, grown))
    }
    /// Byte position AT once every raw-byte character before it is one
    /// byte (`binary`).
    pub(crate) fn raw_compaction(&self, at: usize) -> usize {
        at - self.raw.partition_point(|&p| (p as usize) < at)
    }
    /// The raw-byte offsets of the whole text.
    pub(crate) fn raw_offsets(&self) -> &[u32] {
        &self.raw
    }
    pub fn accessible(&self) -> Range<ByteOffset> {
        self.restriction.clone()
    }
    pub fn narrow(&mut self, r: Range<ByteOffset>) -> Result<()> {
        self.validate_range(&r)?;
        self.point = self.point.clamp(r.start, r.end);
        self.restriction = r;
        Ok(())
    }
    pub fn widen(&mut self) {
        self.restriction = ByteOffset(0)..ByteOffset(self.len_bytes());
    }
    /// Set this view's narrowing and point, where another view's edit of
    /// the shared body moved them (`Vm::family_fetch`).
    pub(crate) fn set_view(&mut self, restriction: Range<ByteOffset>, point: ByteOffset) {
        debug_assert!(restriction.start <= point && point <= restriction.end && restriction.end.0 <= self.len_bytes());
        self.restriction = restriction;
        self.point = point;
    }
    #[inline(never)]
    pub(crate) fn append_utf8(&mut self, s: &str) -> bool {
        if s.len() > 512 || self.point.0 != self.len_bytes() {
            return false;
        }
        let chars = s.chars().count();
        if chars > 128 {
            return false;
        }
        if s.is_empty() {
            return true;
        }
        let at = self.point.0;
        let linecols = self.ts_linecols_before(at..at);
        self.body.append_body(s, chars);
        self.point.0 += s.len();
        self.restriction.end.0 += s.len();
        self.state.record_text_change(chars);
        self.ts_record(at..at, s.len(), linecols);
        true
    }
    pub fn replace(&mut self, r: Range<ByteOffset>, inserted: &[u8]) -> Result<()> {
        self.validate_range(&r)?;
        let (s, local) = internal_to_rope(inserted)?;
        if r.is_empty() && s.is_empty() {
            return Ok(());
        }
        // All retained state is byte-canonical. Conversion is only for index ranks.
        let shift = |p: ByteOffset| {
            ByteOffset(if p < r.start {
                p.0
            } else if p <= r.end {
                r.start.0 + s.len()
            } else {
                p.0 - (r.end.0 - r.start.0) + s.len()
            })
        };
        let linecols = self.ts_linecols_before(r.start.0..r.end.0);
        let changed = self.body.replace_body(&r, &s, &local);
        self.ts_record(r.start.0..r.end.0, s.len(), linecols);
        self.point = shift(self.point);
        let old = self.restriction.clone();
        self.restriction = (if old.start <= r.start {
            old.start
        } else {
            shift(old.start)
        })..shift(old.end);
        self.state.record_text_change(changed);
        Ok(())
    }
    /// Rewrite the one character of R as REPLACEMENT, of another byte
    /// length, its properties stretched over the new text and point and
    /// the restriction kept at their characters. The caller records the
    /// change and moves the anchors (`AnnotatedBuffer::rewrite_character').
    pub(crate) fn rewrite_unanchored(&mut self, r: Range<ByteOffset>, replacement: &[u8]) -> Result<()> {
        self.validate_range(&r)?;
        let (s, local) = internal_to_rope(replacement)?;
        let shift = |p: ByteOffset| if p < r.end { p } else { ByteOffset(p.0 - (r.end.0 - r.start.0) + s.len()) };
        self.point = shift(self.point);
        self.restriction = shift(self.restriction.start)..shift(self.restriction.end);
        let linecols = self.ts_linecols_before(r.start.0..r.end.0);
        let start = r.start.0;
        let old = r.end.0;
        self.body.rewrite_body(r, &s, &local);
        self.ts_record(start..old, s.len(), linecols);
        Ok(())
    }
    /// `TextBody::overwrite' of R, recorded for tree-sitter: editfns.c's
    /// subst-char-in-region and translate-region record each character, one
    /// record of the run is the same edit.
    pub(crate) fn overwrite(&mut self, r: Range<ByteOffset>, replacement: &[u8]) -> Result<()> {
        let linecols = self.ts_linecols_before(r.start.0..r.end.0);
        self.body.overwrite(r.clone(), replacement)?;
        self.ts_record(r.start.0..r.end.0, r.end.0 - r.start.0, linecols);
        Ok(())
    }
    /// The linecols of a change's start and old end in a buffer tracking
    /// them (treesit_linecol_maybe), from the point cache.
    fn ts_linecols_before(&self, r: Range<usize>) -> Option<(crate::treesit::Linecol, crate::treesit::Linecol)> {
        if self.body.treesit.is_empty() || self.ts_linecols[0].bytepos == 0 {
            return None;
        }
        let start = crate::treesit::linecol_of_pos(&self.body, r.start as i64 + 1, self.ts_linecols[1]);
        Some((start, crate::treesit::linecol_of_pos(&self.body, r.end as i64 + 1, start)))
    }
    /// treesit_record_change of the change of OLD to INSERTED bytes, made:
    /// every parser's tree edited, and the linecol caches moved.
    fn ts_record(&mut self, old: Range<usize>, inserted: usize, linecols: Option<(crate::treesit::Linecol, crate::treesit::Linecol)>) {
        if self.body.treesit.is_empty() {
            return;
        }
        let (start, old_end, new_end) = (old.start as i64 + 1, old.end as i64 + 1, (old.start + inserted) as i64 + 1);
        let linecols = linecols.map(|(s, o)| (s, o, crate::treesit::linecol_of_pos(&self.body, new_end, s)));
        self.body.treesit.record(crate::treesit::Change { start, old_end, new_end, linecols }, &self.body);
        if let Some(change) = linecols {
            let (begv, zv) = (self.restriction.start.0 as i64 + 1, self.restriction.end.0 as i64 + 1);
            let body = &*self.body;
            self.ts_linecols = [
                crate::treesit::linecol_by_change(body, self.ts_linecols[0], change, begv),
                change.2,
                crate::treesit::linecol_by_change(body, self.ts_linecols[2], change, zv),
            ];
        }
    }
    /// This view's own values; the body's are its owner's to visit.
    pub(crate) fn visit_view_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.syntax_table);
        visit(&mut self.category_table);
    }
}
impl TextBody {
    /// `Text::append_utf8`'s edit of the body: S, of CHARS characters,
    /// after its end.
    fn append_body(&mut self, s: &str, chars: usize) {
        self.invalidate_reads();
        let end = self.bytes_len;
        self.search.replace(end..end, || Cow::Borrowed(s.as_bytes()));
        if self.tail.len() + s.len() > 1024 {
            self.flush_tail();
        }
        self.tail.push_str(s);
        self.tail_chars += chars;
        self.bytes_len += s.len();
        self.chars_len += chars;
        self.is_ascii &= s.is_ascii();
    }
    /// `Text::replace`'s edit of the body: R replaced by S. Returns the
    /// characters changed, deleted and inserted.
    fn replace_body(&mut self, r: &Range<ByteOffset>, s: &str, local: &[u32]) -> usize {
        self.flush_tail();
        raw_splice(&mut self.raw, r.start.0..r.end.0, s.len(), local);
        self.search.replace(r.start.0..r.end.0, || internal_of(s, local));
        // The edit API already validated UTF-8 boundaries; use the rope's native
        // scalar summaries without perturbing the read-coordinate cursor.
        let start = CharOffset(if self.is_ascii {
            r.start.0
        } else {
            self.rope.byte_to_char(r.start.0)
        });
        let end = CharOffset(if self.is_ascii {
            r.end.0
        } else {
            self.rope.byte_to_char(r.end.0)
        });
        let property_work = self.properties.replace(r.clone(), s.len());
        #[cfg(test)]
        {
            self.property_edit_visits = property_work;
        }
        #[cfg(not(test))]
        let _ = property_work;
        self.invalidate_reads();
        if start != end {
            self.rope.remove(start.0..end.0);
        }
        if !s.is_empty() {
            self.rope.insert(start.0, s);
        }
        self.bytes_len = self.rope.len_bytes();
        self.chars_len = self.rope.len_chars();
        self.is_ascii = self.bytes_len == self.chars_len;
        end.0 - start.0 + s.chars().count()
    }
    /// `Text::rewrite_unanchored`'s edit of the body.
    fn rewrite_body(&mut self, r: Range<ByteOffset>, replacement: &str, local: &[u32]) {
        self.flush_tail();
        raw_splice(&mut self.raw, r.start.0..r.end.0, replacement.len(), local);
        self.search.replace(r.start.0..r.end.0, || internal_of(replacement, local));
        let start = if self.is_ascii { r.start.0 } else { self.rope.byte_to_char(r.start.0) };
        let end = if self.is_ascii { r.end.0 } else { self.rope.byte_to_char(r.end.0) };
        self.properties.resize_character(r, replacement.len());
        self.invalidate_reads();
        self.rope.remove(start..end);
        self.rope.insert(start, replacement);
        self.bytes_len = self.rope.len_bytes();
        self.chars_len = self.rope.len_chars();
        self.is_ascii = self.bytes_len == self.chars_len;
    }
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        self.properties.visit_values(visit);
        self.treesit.visit_values(visit);
    }
    /// The bytes from AT to the end of its rope chunk (or the tail), for a
    /// parse's reader; empty at the end.
    pub(crate) fn chunk_from(&self, at: usize) -> &[u8] {
        let rope_bytes = self.bytes_len - self.tail.len();
        if at >= rope_bytes {
            return self.tail.as_bytes().get(at - rope_bytes..).unwrap_or(&[]);
        }
        let (chunk, start, ..) = self.rope.chunk_at_byte(at);
        &chunk.as_bytes()[at - start..]
    }
    /// The first newline in FROM..LIMIT (0-based bytes).
    pub(crate) fn newline_after(&self, from: usize, limit: usize) -> Option<usize> {
        let mut at = from;
        while at < limit {
            let chunk = self.chunk_from(at);
            let chunk = &chunk[..chunk.len().min(limit - at)];
            if chunk.is_empty() { return None; }
            if let Some(i) = chunk.iter().position(|&b| b == b'\n') { return Some(at + i); }
            at += chunk.len();
        }
        None
    }
    /// The last newline in LIMIT..FROM (0-based bytes).
    pub(crate) fn newline_before(&self, from: usize, limit: usize) -> Option<usize> {
        let mut end = from.min(self.bytes_len);
        while end > limit {
            let (chunk, start) = self.chunk_before(end);
            let start = start.max(limit);
            let slice = &chunk[start - (end - chunk.len())..];
            if let Some(i) = slice.iter().rposition(|&b| b == b'\n') { return Some(start + i); }
            end = start;
        }
        None
    }
    /// The bytes of R, chunk by chunk, read where they lie.
    pub(crate) fn chunks_in(&self, r: Range<usize>) -> impl Iterator<Item = &[u8]> + '_ {
        let mut at = r.start;
        std::iter::from_fn(move || {
            let chunk = self.chunk_from(at);
            let chunk = &chunk[..chunk.len().min(r.end.saturating_sub(at))];
            at += chunk.len();
            (!chunk.is_empty()).then_some(chunk)
        })
    }
    /// The byte where the character before byte AT starts; 0 at the start.
    pub(crate) fn char_start_before(&self, at: usize) -> usize {
        // A chunk never splits a character: scan back within the one
        // holding the byte before AT.
        let start = |bytes: &[u8], base: usize| {
            let mut i = at - 1 - base;
            while i > 0 && bytes[i] & 0xc0 == 0x80 {
                i -= 1;
            }
            base + i
        };
        let rope_bytes = self.bytes_len - self.tail.len();
        match self.flat.get() {
            _ if at == 0 => 0,
            Some(flat) => start(flat.as_bytes(), 0),
            None if at > rope_bytes => start(self.tail.as_bytes(), rope_bytes),
            None => {
                let cursor = self.chunk_for_byte(at - 1);
                start(cursor.text.as_bytes(), cursor.byte)
            }
        }
    }
    /// The bytes of the chunk holding the byte before END, up to END, and
    /// where that chunk starts.
    fn chunk_before(&self, end: usize) -> (&[u8], usize) {
        let rope_bytes = self.bytes_len - self.tail.len();
        if end > rope_bytes {
            return (&self.tail.as_bytes()[..end - rope_bytes], rope_bytes);
        }
        let (chunk, start, ..) = self.rope.chunk_at_byte(end - 1);
        (&chunk.as_bytes()[..end - start], start)
    }
    pub fn as_str(&self) -> &str {
        self.flat.get_or_init(|| {
            count_copied(self.bytes_len);
            let mut text = self.rope.to_string();
            text.push_str(&self.tail);
            text
        })
    }
    /// Raw stored annotations, including explicit nil; no visual defaults.
    #[cfg(test)]
    pub(crate) fn property(&self, at: ByteOffset, key: &str) -> Result<Option<Value>> {
        self.boundary(at)?;
        Ok(self.properties.get_property(at, key))
    }
    #[cfg(test)]
    pub(crate) fn put_property(
        &mut self,
        range: Range<ByteOffset>,
        key: &str,
        value: Value,
    ) -> Result<()> {
        self.validate_range(&range)?;
        self.properties.put(range, key, value);
        Ok(())
    }
    pub fn remove_property(&mut self, range: Range<ByteOffset>, key: &str) -> Result<()> {
        self.validate_range(&range)?;
        self.properties.remove(range, key);
        Ok(())
    }
    pub fn next_property_change(&self, at: ByteOffset, limit: ByteOffset) -> Result<ByteOffset> {
        self.validate_range(&(at..limit))?;
        Ok(self.properties.next_change(at, limit))
    }
    /// The text in GNU's internal form (raw bytes as C0/C1 forms).
    pub fn bytes(&self) -> &[u8] {
        if self.raw.is_empty() {
            return self.as_str().as_bytes();
        }
        self.flat_internal.get_or_init(|| { count_copied(self.bytes_len); rope_to_internal(self.as_str().as_bytes(), 0, &self.raw) })
    }
    /// A run of the text that holds NEED and lies within BOUNDS (byte
    /// ranges) and that [`Self::search_bytes`] reads contiguously, in GNU's
    /// internal form, for a search: the flat text when it is current;
    /// otherwise the search view, built once and then kept up to date by
    /// each edit, so a search after an edit costs what it reads
    /// (text_gap.rs).
    pub(crate) fn search_window(&mut self, need: Range<usize>, bounds: Range<usize>) -> Range<usize> {
        if self.flat_current() {
            return bounds;
        }
        let view = match &mut self.search.0 {
            Some(view) => view,
            empty => empty.insert(gap::GapText::new(
                self.rope.chunks().chain(std::iter::once(self.tail.as_str())).map(str::as_bytes),
                self.bytes_len,
                &self.raw,
            )),
        };
        view.window(need, bounds)
    }
    /// The bytes of WINDOW, a run [`Self::search_window`] gave since the
    /// last edit.
    pub(crate) fn search_bytes(&self, window: Range<usize>) -> &[u8] {
        match &self.search.0 {
            Some(view) if !self.flat_current() => view.slice(window),
            _ => &self.bytes()[window],
        }
    }
    /// Whether [`Self::bytes`] is built and current.
    fn flat_current(&self) -> bool {
        if self.raw.is_empty() { self.flat.get().is_some() } else { self.flat_internal.get().is_some() }
    }
    /// R of the text in GNU's internal form; borrowed unless a raw byte
    /// is in it.
    pub fn internal(&self, r: Range<ByteOffset>) -> Result<Cow<'_, [u8]>> {
        let text = self.slice(r.clone())?;
        if raw_in(&self.raw, r.start.0..r.end.0).is_empty() {
            return Ok(match text {
                Cow::Borrowed(s) => Cow::Borrowed(s.as_bytes()),
                Cow::Owned(s) => Cow::Owned(s.into_bytes()),
            });
        }
        Ok(Cow::Owned(rope_to_internal(text.as_bytes(), r.start.0, &self.raw)))
    }
    /// R of the text as a multibyte Lisp string with its properties, raw
    /// bytes kept.
    pub(crate) fn lisp_string(&self, r: Range<ByteOffset>) -> Result<crate::data::LispString> {
        let mut s = crate::data::LispString::from_bytes(self.internal(r.clone())?.into_owned(), true);
        s.properties = self.properties.slice(r);
        Ok(s)
    }
    /// Whether any raw byte is in the text.
    #[inline]
    pub fn has_raw(&self) -> bool {
        !self.raw.is_empty()
    }
    /// Whether the character starting at byte B is a raw byte.
    #[inline]
    pub fn is_raw_at(&self, b: usize) -> bool {
        !self.raw.is_empty() && self.raw.binary_search(&(b as u32)).is_ok()
    }
    /// The raw-byte offsets inside R.
    pub fn raw_in(&self, r: Range<usize>) -> &[u32] {
        raw_in(&self.raw, r)
    }
    /// The character code of the scalar CH read at byte B: a placeholder
    /// listed in `raw` is its raw byte.
    #[inline]
    pub fn code_at(&self, b: usize, ch: char) -> u32 {
        let code = u32::from(ch);
        if (0x80..=0xff).contains(&code) && self.is_raw_at(b) { code + crate::lstring::BYTE8_OFFSET } else { code }
    }
    pub fn len_chars(&self) -> usize {
        self.chars_len
    }
    pub fn len_bytes(&self) -> usize {
        self.bytes_len
    }
    pub fn is_ascii(&self) -> bool {
        self.is_ascii
    }
    #[inline]
    pub fn byte_of_char(&self, c: CharOffset) -> Result<ByteOffset> {
        if c.0 > self.len_chars() {
            return Err("index out of range".into());
        }
        if self.is_ascii {
            return Ok(ByteOffset(c.0));
        }
        let prefix_chars = self.chars_len - self.tail_chars;
        if c.0 >= prefix_chars {
            return Ok(ByteOffset(
                self.bytes_len - self.tail.len()
                    + str_indices::chars::to_byte_idx(&self.tail, c.0 - prefix_chars),
            ));
        }
        if c.0 == self.len_chars() {
            return Ok(ByteOffset(self.len_bytes()));
        }
        let (prior_char, prior_byte) = self.coordinate.get();
        if c.0 == prior_char {
            return Ok(ByteOffset(prior_byte));
        }
        let chunk = self.chunk_for_char(c.0);
        let local = c.0 - chunk.scalar;
        let (at, scanned) = if chunk.text.len() == chunk.chars {
            (local, 0)
        } else if c.0 > prior_char
            && c.0 - prior_char <= 8
            && prior_byte >= chunk.byte
            && prior_byte < chunk.byte + chunk.text.len()
        {
            let origin = prior_byte - chunk.byte;
            let at =
                origin + str_indices::chars::to_byte_idx(&chunk.text[origin..], c.0 - prior_char);
            (at, at - origin)
        } else {
            chunk.byte_of_char(local)
        };
        #[cfg(test)]
        self.scanned_bytes.set(self.scanned_bytes.get() + scanned);
        #[cfg(not(test))]
        let _ = scanned;
        self.coordinate.set((c.0, chunk.byte + at));
        Ok(ByteOffset(chunk.byte + at))
    }
    pub fn char_of_byte(&self, b: ByteOffset) -> Result<CharOffset> {
        if b.0 > self.bytes_len {
            return Err("index out of range".into());
        }
        if self.is_ascii {
            return Ok(CharOffset(b.0));
        }
        let (prior_char, prior_byte) = self.coordinate.get();
        if b.0 == prior_byte {
            return Ok(CharOffset(prior_char));
        }
        self.boundary(b)?;
        let prefix_bytes = self.bytes_len - self.tail.len();
        if b.0 >= prefix_bytes {
            return Ok(CharOffset(
                self.chars_len - self.tail_chars
                    + str_indices::chars::from_byte_idx(&self.tail, b.0 - prefix_bytes),
            ));
        }
        let chunk = self.chunk_for_byte(b.0);
        let local = b.0 - chunk.byte;
        let at = chunk.scalar + str_indices::chars::from_byte_idx(&chunk.text, local);
        #[cfg(test)]
        self.scanned_bytes.set(self.scanned_bytes.get() + local);
        self.coordinate.set((at, b.0));
        Ok(CharOffset(at))
    }
    fn promote_chunk(&self, c: usize) -> bool {
        let mut cold = self.cold_chunks.borrow_mut();
        let Some((&key, cached)) = cold.range(..=c).next_back() else {
            return false;
        };
        if c >= cached.scalar + cached.chars {
            return false;
        }
        let cached = cold.remove(&key).unwrap();
        let mut order = self.cold_order.borrow_mut();
        let index = order
            .iter()
            .position(|&k| k == key)
            .expect("cached chunk queued");
        order.remove(index);
        let previous = self.chunk.replace(cached);
        if !previous.text.is_empty() {
            order.push_back(previous.scalar);
            cold.insert(previous.scalar, previous);
        }
        true
    }
    fn retain_chunk(&self) {
        let previous = self.chunk.take();
        if !previous.text.is_empty() {
            let mut cold = self.cold_chunks.borrow_mut();
            let mut order = self.cold_order.borrow_mut();
            if cold.len() == 31 {
                *self.chunk.borrow_mut() = cold.remove(&order.pop_front().unwrap()).unwrap();
            }
            order.push_back(previous.scalar);
            cold.insert(previous.scalar, previous);
        }
    }
    fn cache_chunk(&self, text: &str, byte: usize, scalar: usize) {
        self.chunk.borrow_mut().fill(text, byte, scalar);
        #[cfg(test)]
        self.coordinate_steps.set(self.coordinate_steps.get() + 2);
    }
    fn chunk_for_char(&self, c: usize) -> std::cell::Ref<'_, ChunkCursor> {
        let hit = {
            let cur = self.chunk.borrow();
            c >= cur.scalar && c < cur.scalar + cur.chars
        };
        if !hit && !self.promote_chunk(c) {
            self.retain_chunk();
            let (s, b, c, _) = self.rope.chunk_at_char(c);
            self.cache_chunk(s, b, c);
        }
        self.chunk.borrow()
    }
    fn chunk_for_byte(&self, b: usize) -> std::cell::Ref<'_, ChunkCursor> {
        let hit = {
            let cur = self.chunk.borrow();
            b >= cur.byte && b < cur.byte + cur.text.len()
        };
        if !hit {
            let cached = self
                .cold_chunks
                .borrow()
                .values()
                .find(|cur| b >= cur.byte && b < cur.byte + cur.text.len())
                .map(|cur| cur.scalar);
            if !cached.is_some_and(|c| self.promote_chunk(c)) {
                self.retain_chunk();
                let (s, b, c, _) = self.rope.chunk_at_byte(b);
                self.cache_chunk(s, b, c);
            }
        }
        self.chunk.borrow()
    }
    fn invalidate_reads(&mut self) {
        self.flat.take();
        self.flat_internal.take();
        self.cold_chunks.get_mut().clear();
        self.cold_order.get_mut().clear();
        self.coordinate.set((0, 0));
        let cur = self.chunk.get_mut();
        cur.text.clear();
        cur.byte = 0;
        cur.scalar = 0;
        cur.chars = 0;
    }
    fn boundary(&self, b: ByteOffset) -> Result<()> {
        if b.0 > self.len_bytes() {
            return Err("index out of range".into());
        }
        if self.is_ascii || b.0 == self.len_bytes() {
            return Ok(());
        }
        if b.0 >= self.bytes_len - self.tail.len() {
            return if self
                .tail
                .is_char_boundary(b.0 - (self.bytes_len - self.tail.len()))
            {
                Ok(())
            } else {
                Err("index out of range".into())
            };
        }
        let cursor = self.chunk.borrow();
        let valid = if b.0 >= cursor.byte && b.0 < cursor.byte + cursor.text.len() {
            cursor.text.is_char_boundary(b.0 - cursor.byte)
        } else {
            let (chunk, start, _, _) = self.rope.chunk_at_byte(b.0);
            chunk.is_char_boundary(b.0 - start)
        };
        if valid {
            Ok(())
        } else {
            Err("index out of range".into())
        }
    }
    #[inline]
    pub fn character_at(&self, b: ByteOffset) -> Result<Option<i64>> {
        if b.0 > self.len_bytes() {
            return Err("index out of range".into());
        }
        if b.0 == self.len_bytes() {
            return Ok(None);
        }
        if b.0 >= self.bytes_len - self.tail.len() {
            return self
                .tail
                .get(b.0 - (self.bytes_len - self.tail.len())..)
                .map(|s| s.chars().next().map(|c| i64::from(self.code_at(b.0, c))))
                .ok_or_else(|| "index out of range".into());
        }
        let chunk = self.chunk_for_byte(b.0);
        let ch = chunk
            .text
            .get(b.0 - chunk.byte..)
            .ok_or_else(|| "index out of range".to_owned())?
            .chars()
            .next()
            .unwrap();
        let (scalar, byte) = self.coordinate.get();
        if byte == b.0 {
            self.coordinate.set((scalar + 1, byte + ch.len_utf8()));
        }
        Ok(Some(i64::from(self.code_at(b.0, ch))))
    }
    /// The character code at character C. Scalar lookup keeps conversion
    /// and decoding in one chunk borrow.
    #[inline(always)]
    pub(crate) fn character_of_char(&self, c: CharOffset) -> Result<Option<u32>> {
        if c.0 > self.chars_len {
            return Err("index out of range".into());
        }
        if c.0 == self.chars_len {
            return Ok(None);
        }
        if c.0 >= self.chars_len - self.tail_chars {
            let prefix = self.bytes_len - self.tail.len();
            let at = str_indices::chars::to_byte_idx(
                &self.tail,
                c.0 - (self.chars_len - self.tail_chars),
            );
            return Ok(self.tail[at..].chars().next().map(|ch| self.code_at(prefix + at, ch)));
        }
        let chunk = self.chunk_for_char(c.0);
        if chunk.chars == chunk.text.len() {
            return Ok(Some(u32::from(chunk.text.as_bytes()[c.0 - chunk.scalar])));
        }
        let (prior_char, prior_byte) = self.coordinate.get();
        let at = if c.0 == prior_char
            && prior_byte >= chunk.byte
            && prior_byte < chunk.byte + chunk.text.len()
        {
            prior_byte - chunk.byte
        } else {
            chunk.byte_of_char(c.0 - chunk.scalar).0
        };
        let ch = chunk.text[at..].chars().next().unwrap();
        self.coordinate
            .set((c.0 + 1, chunk.byte + at + ch.len_utf8()));
        Ok(Some(self.code_at(chunk.byte + at, ch)))
    }
    /// R of the text: borrowed from the flat text when it is current or
    /// from the one chunk that holds R, else copied from its chunks, so a
    /// read after an edit costs R's length, not the text's.
    pub fn slice(&self, r: Range<ByteOffset>) -> Result<Cow<'_, str>> {
        self.validate_range(&r)?;
        let (start, end) = (r.start.0, r.end.0);
        if let Some(flat) = self.flat.get() {
            return Ok(Cow::Borrowed(&flat[start..end]));
        }
        let first = self.chunk_from(start);
        let bytes = if first.len() >= end - start {
            Cow::Borrowed(&first[..end - start])
        } else {
            Cow::Owned(self.chunks_in(start..end).collect::<Vec<_>>().concat())
        };
        // Both ends are character boundaries (validate_range).
        Ok(match bytes {
            Cow::Borrowed(b) => Cow::Borrowed(std::str::from_utf8(b).map_err(|e| e.to_string())?),
            Cow::Owned(b) => Cow::Owned(String::from_utf8(b).map_err(|e| e.to_string())?),
        })
    }
    pub(crate) fn find_character(
        &self,
        ch: u32,
        from: ByteOffset,
        to: ByteOffset,
    ) -> Option<ByteOffset> {
        // A raw byte is found at its placeholder, which also stands for
        // the scalar of the same code elsewhere: the listing decides.
        let raw = ch > crate::lstring::MAX_5_BYTE_CHAR;
        let needle = char::from_u32(if raw { ch - crate::lstring::BYTE8_OFFSET } else { ch })?;
        let text = self.slice(from..to).ok()?;
        let mut at = text.find(needle)?;
        while (0x80..=0xff).contains(&u32::from(needle)) && self.is_raw_at(from.0 + at) != raw {
            let next = at + needle.len_utf8();
            at = next + text[next..].find(needle)?;
        }
        let (scalar, byte) = self.coordinate.get();
        if byte == from.0 {
            self.coordinate.set((
                scalar + text[..at + needle.len_utf8()].chars().count(),
                from.0 + at + needle.len_utf8(),
            ));
        }
        Some(ByteOffset(from.0 + at))
    }
    fn validate_range(&self, r: &Range<ByteOffset>) -> Result<()> {
        if r.start > r.end {
            return Err("index out of range".into());
        }
        self.boundary(r.start)?;
        self.boundary(r.end)
    }
    /// LF-based byte coordinates; CR is ordinary text.
    pub fn line_of_byte(&self, b: ByteOffset) -> Result<usize> {
        self.boundary(b)?;
        let prefix_bytes = self.bytes_len - self.tail.len();
        if b.0 >= prefix_bytes {
            Ok(self.rope.len_lines() - 1
                + self.tail.as_bytes()[..b.0 - prefix_bytes]
                    .iter()
                    .filter(|&&c| c == b'\n')
                    .count())
        } else {
            Ok(self.rope.byte_to_line(b.0))
        }
    }
    pub fn byte_of_line(&self, line: usize) -> Result<ByteOffset> {
        let prefix_lines = self.rope.len_lines() - 1;
        if line <= prefix_lines {
            return self
                .rope
                .try_line_to_byte(line)
                .map(ByteOffset)
                .map_err(|_| "index out of range".into());
        }
        let at = self
            .tail
            .as_bytes()
            .iter()
            .enumerate()
            .filter(|&(_, &c)| c == b'\n')
            .nth(line - prefix_lines - 1)
            .map(|(b, _)| ByteOffset(self.bytes_len - self.tail.len() + b + 1));
        at.ok_or_else(|| "index out of range".into())
    }
    pub fn chunks(&self) -> impl Iterator<Item = &str> {
        self.rope
            .chunks()
            .chain((!self.tail.is_empty()).then_some(self.tail.as_str()))
    }
    fn flush_tail(&mut self) {
        if !self.tail.is_empty() {
            self.rope
                .insert(self.chars_len - self.tail_chars, &self.tail);
            self.tail.clear();
            self.tail_chars = 0;
        }
    }
    /// Overwrite the characters of R with as many characters of the same
    /// total byte length, keeping text properties, point and markers, as
    /// GNU's subst-char-in-region does. The caller records the change.
    pub(crate) fn overwrite(&mut self, r: Range<ByteOffset>, replacement: &[u8]) -> Result<()> {
        self.validate_range(&r)?;
        let (replacement, local) = internal_to_rope(replacement)?;
        if replacement.len() != r.end.0 - r.start.0 { return Err("overwrite length differs".into()); }
        self.flush_tail();
        raw_splice(&mut self.raw, r.start.0..r.end.0, replacement.len(), &local);
        self.search.replace(r.start.0..r.end.0, || internal_of(&replacement, &local));
        let start = if self.is_ascii { r.start.0 } else { self.rope.byte_to_char(r.start.0) };
        let end = if self.is_ascii { r.end.0 } else { self.rope.byte_to_char(r.end.0) };
        self.invalidate_reads();
        self.rope.remove(start..end);
        self.rope.insert(start, &replacement);
        self.chars_len = self.rope.len_chars();
        self.is_ascii = self.bytes_len == self.chars_len;
        Ok(())
    }
    /// Cursor movement is grapheme based, independently of Lisp scalar indexing.
    pub fn next_grapheme(&self, at: ByteOffset) -> Result<ByteOffset> {
        self.boundary(at)?;
        // Segment the whole text: a scalar boundary can be inside a cluster.
        Ok(self
            .as_str()
            .grapheme_indices(true)
            .map(|(b, g)| b + g.len())
            .find(|&b| b > at.0)
            .map(ByteOffset)
            .unwrap_or_else(|| ByteOffset(self.len_bytes())))
    }
    pub fn previous_grapheme(&self, at: ByteOffset) -> Result<ByteOffset> {
        self.boundary(at)?;
        Ok(ByteOffset(
            self.as_str()
                .grapheme_indices(true)
                .map(|(b, _)| b)
                .take_while(|&b| b < at.0)
                .last()
                .unwrap_or(0),
        ))
    }
    pub fn grapheme_offset(&self, at: ByteOffset) -> Result<GraphemeOffset> {
        self.boundary(at)?;
        if at.0 != self.len_bytes() && !self.as_str().grapheme_indices(true).any(|(b, _)| b == at.0)
        {
            return Err("not a grapheme boundary".into());
        }
        Ok(GraphemeOffset(
            self.as_str()[..at.0].graphemes(true).count(),
        ))
    }
    pub(crate) fn storage_bytes(&self, capacity: bool) -> usize {
        let properties = self.properties.storage_bytes();
        // Pinned Ropey 1.6.1 allocates exactly 1024 bytes per Arc<Node>
        // (its tree/mod.rs has a compile-time assertion). Its public capacity()
        // counts only text slots, not branch nodes. With L leaves, a tree whose
        // branches have at least two children has at most 2L-1 total nodes.
        // Charge that conservative upper bound, including shared nodes in each
        // clone, rather than silently dropping branch/Arc storage from GC stats.
        // Allocator bookkeeping is excluded, as it was for String/AVL storage.
        let leaves = self.rope.chunks().count().max(1);
        let rope_storage_bound = (2 * leaves - 1) * 1024;
        properties
            + self
                .cold_chunks
                .borrow()
                .values()
                .map(|c| c.text.capacity() + std::mem::size_of::<ChunkCursor>() + 64)
                .sum::<usize>()
            + self.cold_order.borrow().capacity() * std::mem::size_of::<usize>()
            + self.tail.capacity()
            + self.chunk.borrow().text.capacity()
            + rope_storage_bound
            + self.raw.capacity() * std::mem::size_of::<u32>()
            + self.search.storage_bytes(capacity)
            + self.flat_internal.get().map_or(0, |b| if capacity { b.capacity() } else { b.len() })
            + self
                .flat
                .get()
                .map_or(0, |s| if capacity { s.capacity() } else { s.len() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use romacs_coordinate_contract::{self as contract, *};
    #[test]
    fn modification_tracks_edits_noops_failures_and_representation_changes() {
        let mut text = crate::text::Text::new(b"").unwrap();
        let initial = *text.state.modification;
        assert!(text.append_utf8(""));
        text.replace(ByteOffset(0)..ByteOffset(0), b"").unwrap();
        assert_eq!(*text.state.modification, initial);
        assert!(text.replace(ByteOffset(0)..ByteOffset(0), &[0xff]).is_err());
        assert_eq!(*text.state.modification, initial);
        assert!(text.append_utf8("λ"));
        assert_eq!(text.state.modification.modiff, 2);
        assert_eq!(text.state.modification.chars_modiff, 2);
        // Replacing identical bytes is still a delete/insert operation.
        text.replace(ByteOffset(0)..ByteOffset(2), "λ".as_bytes()).unwrap();
        assert_eq!(text.state.modification.modiff, 4);
        let before = *text.state.modification;
        assert!(text.replace(ByteOffset(1)..ByteOffset(2), b"").is_err());
        assert_eq!(*text.state.modification, before);
        let mut binary = text.binary();
        assert_eq!(*binary.state.modification, before);
        binary.replace(ByteOffset(0)..ByteOffset(0), b"").unwrap();
        assert_eq!(*binary.state.modification, before);
        binary.replace(ByteOffset(0)..ByteOffset(2), b"abcd").unwrap();
        assert_eq!(binary.state.modification.modiff, 7);
        assert_eq!(binary.state.modification.chars_modiff, 7);
        binary.state.modification.set_modified(false);
        binary.state.modification.autosave_modiff = 8;
        let (decoded, _) = crate::text::Text::decode_binary(&binary).unwrap();
        assert_eq!(*decoded.state.modification, *binary.state.modification);
        let wrapped: crate::annotations::AnnotatedBuffer = decoded.into();
        assert_eq!(*wrapped.state.modification, *binary.state.modification);
        assert_eq!(*wrapped.clone().state.modification, *binary.state.modification);
    }
    // Adapt the legacy untyped acceptance seam at one explicit boundary.
    struct Text(crate::text::Text);
    impl std::ops::Deref for Text {
        type Target = crate::text::Text;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
    impl std::ops::DerefMut for Text {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }
    impl Text {
        fn new(bytes: &[u8]) -> Result<Self> {
            crate::text::Text::new(bytes).map(Self)
        }
        fn len_bytes(&self) -> usize {
            self.0.len_bytes()
        }
        fn len_chars(&self) -> usize {
            self.0.len_chars()
        }
        fn widen(&mut self) {
            self.0.widen()
        }
        fn byte_of_char(&self, c: usize) -> Result<usize> {
            self.0.byte_of_char(CharOffset(c)).map(|b| b.0)
        }
        fn char_of_byte(&self, b: usize) -> Result<usize> {
            self.0.char_of_byte(ByteOffset(b)).map(|c| c.0)
        }
        fn character_at(&self, c: usize) -> Result<Option<i64>> {
            self.0.character_at(self.0.byte_of_char(CharOffset(c))?)
        }
        fn slice(&self, r: Range<usize>) -> Result<Vec<u8>> {
            self.0
                .slice(
                    self.0.byte_of_char(CharOffset(r.start))?
                        ..self.0.byte_of_char(CharOffset(r.end))?,
                )
                .map(|s| s.as_bytes().to_vec())
        }
        fn replace(&mut self, r: Range<usize>, b: &[u8]) -> Result<()> {
            self.0.replace(ByteOffset(r.start)..ByteOffset(r.end), b)
        }
        fn narrow(&mut self, r: Range<usize>) -> Result<()> {
            let a = self.0.byte_of_char(CharOffset(r.start))?;
            let b = self.0.byte_of_char(CharOffset(r.end))?;
            self.0.narrow(a..b)
        }
        fn accessible(&self) -> Range<usize> {
            let r = self.0.accessible();
            self.char_of_byte(r.start.0).unwrap()..self.char_of_byte(r.end.0).unwrap()
        }
        fn append_chars(&mut self, chars: &[char]) -> bool {
            self.0.append_utf8(&chars.iter().collect::<String>())
        }
    }

    fn bad<T>(r: Result<T>) -> std::result::Result<T, InvalidPosition> {
        r.map_err(|_| InvalidPosition)
    }
    impl Coordinates for Text {
        fn len_bytes(&self) -> usize {
            Text::len_bytes(self)
        }
        fn len_chars(&self) -> usize {
            Text::len_chars(self)
        }
        fn byte_of_char(&self, c: usize) -> std::result::Result<usize, InvalidPosition> {
            bad(Text::byte_of_char(self, c))
        }
        fn char_of_byte(&self, b: usize) -> std::result::Result<usize, InvalidPosition> {
            bad(Text::char_of_byte(self, b))
        }
        fn replace(
            &mut self,
            r: Range<usize>,
            b: &[u8],
        ) -> std::result::Result<(), InvalidPosition> {
            bad(Text::replace(self, r, b))
        }
        fn set_mode(&mut self, m: Mode) {
            assert_eq!(m, Mode::Multibyte, "text cannot reinterpret bytes")
        }
    }
    impl RestrictedCoordinates for Text {
        fn narrow(&mut self, r: Range<usize>) -> std::result::Result<(), InvalidPosition> {
            bad(Text::narrow(self, r))
        }
        fn widen(&mut self) {
            Text::widen(self)
        }
        fn accessible_chars(&self) -> Range<usize> {
            self.accessible()
        }
    }
    impl EditorCoordinates for Text {
        fn mode(&self) -> Mode {
            Mode::Multibyte
        }
        fn character_at(&self, c: usize) -> std::result::Result<Option<i64>, InvalidPosition> {
            bad(Text::character_at(self, c))
        }
        fn slice_chars(&self, r: Range<usize>) -> std::result::Result<Vec<u8>, InvalidPosition> {
            bad(self.slice(r).map(Vec::from))
        }
        fn put_property(
            &mut self,
            r: Range<usize>,
            k: &str,
            v: i64,
        ) -> std::result::Result<(), InvalidPosition> {
            bad(self.slice(r.clone()))?;
            for p in r {
                let b = ByteOffset(self.byte_of_char(p).unwrap());
                let end = ByteOffset(self.byte_of_char(p + 1).unwrap());
                self.properties.put(b..end, k, Value::integer(v).unwrap());
            }
            Ok(())
        }
        fn property_at(
            &self,
            p: usize,
            k: &str,
        ) -> std::result::Result<Option<i64>, InvalidPosition> {
            bad(Text::character_at(self, p))?;
            Ok(self
                .properties
                .get_property(ByteOffset(self.byte_of_char(p).unwrap()), k)
                .map(|v| v.as_integer().unwrap()))
        }
    }
    impl MeasuredCoordinates for Text {
        fn coordinate_work(&self) -> CoordinateWork {
            CoordinateWork {
                scanned_bytes: self.scanned_bytes.get(),
                tree_steps: self.coordinate_steps.get(),
            }
        }
    }
    fn make(b: &[u8], _m: Mode) -> Text {
        Text::new(b).unwrap()
    }
    #[test]
    fn bounded_chunk_cursor_is_invalidated_by_mutation_and_copy_is_independent() {
        let source = "aλЖ🙂z\n".repeat(4096);
        let mut text = crate::text::Text::new(source.as_bytes()).unwrap();
        for (c, (b, ch)) in source.char_indices().enumerate() {
            assert_eq!(text.byte_of_char(CharOffset(c)), Ok(ByteOffset(b)));
            assert_eq!(text.character_at(ByteOffset(b)), Ok(Some(ch as i64)));
        }
        assert!(
            text.chunk.borrow().text.capacity() <= 2048,
            "read cursor grew beyond one rope chunk"
        );
        let copy = text.clone();
        text.replace(ByteOffset(0)..ByteOffset(1), "🙂".as_bytes())
            .unwrap();
        assert!(text.chunk.borrow().text.is_empty());
        assert_eq!(copy.as_str(), source);
        assert_eq!(text.character_at(ByteOffset(0)), Ok(Some('🙂' as i64)));
        assert_eq!(text.byte_of_char(CharOffset(1)), Ok(ByteOffset(4)));
        assert_eq!(copy.byte_of_char(CharOffset(1)), Ok(ByteOffset(1)));
    }
    #[test]
    fn sparse_property_insertion_does_not_rewrite_the_suffix() {
        let mut text = crate::text::Text::new(&vec![b'a'; 8192]).unwrap();
        for at in (0..8192).step_by(8) {
            text.properties
                .put(ByteOffset(at)..ByteOffset(at + 1), "x", Value::TRUE);
        }
        text.replace(ByteOffset(0)..ByteOffset(0), b"prefix")
            .unwrap();
        assert!(
            text.property_edit_visits < 64,
            "property insertion rewrote the sparse suffix: {} visits",
            text.property_edit_visits
        );
        for at in (0..8192).step_by(8) {
            assert_eq!(
                text.properties.get_property(ByteOffset(at + 6), "x"),
                Some(Value::TRUE)
            );
        }
    }
    #[test]
    fn rope_edits_do_not_materialize_the_compatibility_view() {
        let mut text = crate::text::Text::new("aλЖ🙂\n".repeat(4096).as_bytes()).unwrap();
        for _ in 0..100 {
            text.replace(ByteOffset(1)..ByteOffset(3), "λ".as_bytes())
                .unwrap();
            text.narrow(ByteOffset(0)..ByteOffset(10)).unwrap();
            assert!(
                text.flat.get().is_none(),
                "editing/narrowing flattened the rope"
            );
            text.widen();
        }
        let old = text.as_str().to_owned();
        assert!(text.flat.get().is_some());
        text.replace(ByteOffset(0)..ByteOffset(1), "🙂".as_bytes())
            .unwrap();
        assert!(
            text.flat.get().is_none(),
            "mutation retained stale contiguous bytes"
        );
        assert_eq!(text.as_str(), format!("🙂{}", &old[1..]));
    }
    #[test]
    fn rope_line_summaries_use_lf_and_survive_width_changing_edits() {
        let mut text = crate::text::Text::new("a\rλ\nЖ🙂\r\nlast\n".as_bytes()).unwrap();
        for inserted in ["", "🙂\nλ", "x\r\ny"] {
            text.replace(ByteOffset(0)..ByteOffset(0), inserted.as_bytes())
                .unwrap();
            let expected = text.as_str();
            for (b, _) in expected
                .char_indices()
                .chain(std::iter::once((expected.len(), '\0')))
            {
                let prefix = &expected[..b];
                let line = prefix.bytes().filter(|&x| x == b'\n').count();
                assert_eq!(text.line_of_byte(ByteOffset(b)), Ok(line));
                let start = prefix.rfind('\n').map_or(0, |i| i + 1);
                assert_eq!(text.byte_of_line(line), Ok(ByteOffset(start)));
            }
            assert!(text.line_of_byte(ByteOffset(text.len_bytes() + 1)).is_err());
        }
    }
    #[test]
    fn text_storage_does_not_duplicate_a_dense_scalar_directory() {
        let input = "aλЖ🙂z\n".repeat(100_000);
        let text = crate::text::Text::new(input.as_bytes()).unwrap();
        assert!(
            text.storage_bytes(true) < input.len() * 5,
            "text storage capacity exceeds five times UTF-8 input"
        );
    }
    #[test]
    fn large_index_edits_preserve_avl_summaries_and_relative_offsets() {
        let mut t = Text::new("aλЖ🙂z".repeat(4096).as_bytes()).unwrap();
        let mut chars: Vec<char> = "aλЖ🙂z".repeat(4096).chars().collect();
        let mut seed = 1u64;
        let long = "λЖ".repeat(100);
        for i in 0..1000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let at = seed as usize % (chars.len() + 1);
            let end = (at + ((seed >> 16) as usize % 200)).min(chars.len());
            let added = ["", "a", "ΩΩ🙂", "ascii", long.as_str()][(seed >> 32) as usize % 5];
            let a = t.byte_of_char(at).unwrap();
            let b = t.byte_of_char(end).unwrap();
            t.replace(a..b, added.as_bytes()).unwrap();
            chars.splice(at..end, added.chars());
            assert_eq!(t.len_chars(), chars.len());
            if i % 25 == 0 {
                let text: String = chars.iter().collect();
                assert_eq!(t.bytes(), text.as_bytes());
                for (c, (b, _)) in text.char_indices().enumerate() {
                    assert_eq!(t.byte_of_char(c), Ok(b));
                    assert_eq!(t.char_of_byte(b), Ok(c));
                }
            }
        }
    }
    #[test]
    fn random_edits_match_character_oracle_and_invalid_bytes_are_atomic() {
        let mut t = Text::new(b"").unwrap();
        let mut expected = String::new();
        let mut state = 1u64;
        for _ in 0..1000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let len = expected.chars().count();
            let at = state as usize % (len + 1);
            let end = (at + ((state >> 16) as usize % 4)).min(len);
            let s = ["", "a", "λЖ", "🙂", "abcdef"][(state >> 32) as usize % 5];
            let mut offsets: Vec<_> = expected.char_indices().map(|(b, _)| b).collect();
            offsets.push(expected.len());
            let range = offsets[at]..offsets[end];
            t.replace(range.clone(), s.as_bytes()).unwrap();
            expected.replace_range(range, s);
            assert_eq!(t.bytes(), expected.as_bytes());
            contract::assert_coordinates(&t, expected.as_bytes(), Mode::Multibyte);
        }
        let before = t.bytes().to_vec();
        for bad in [
            &[0x80][..],
            &[0xc2],
            &[0xe0, 0x80, 0x80],
            &[0xff],
            &[0xf8, 0xbf, 0xbf, 0xbf, 0xbf],
        ] {
            assert!(t.replace(0..0, bad).is_err());
            assert_eq!(t.bytes(), before);
            assert!(Text::new(bad).is_err());
        }
        assert!(Text::new(&[0xff]).is_err());
        // GNU's raw-byte forms are text: the bytes 0x80 and 0xff.
        let raw = crate::text::Text::new(&[0xc0, 0x80, 0xc1, 0xbf]).unwrap();
        assert_eq!(raw.len_chars(), 2);
        assert_eq!(raw.bytes(), &[0xc0, 0x80, 0xc1, 0xbf]);
    }

    #[test]
    fn raw_bytes_keep_gnu_positions_and_round_trip() {
        // "a" raw-E9 "é" "b": GNU's internal bytes, placeholders in the rope.
        let internal: &[u8] = &[b'a', 0xc1, 0xa9, 0xc3, 0xa9, b'b'];
        let mut t = crate::text::Text::new(internal).unwrap();
        assert_eq!(t.as_str(), "aééb");
        assert_eq!(t.bytes(), internal);
        assert_eq!(t.len_chars(), 4);
        assert!(t.has_raw() && t.is_raw_at(1) && !t.is_raw_at(3));
        assert_eq!(t.character_at(ByteOffset(1)), Ok(Some(0x3fffe9)));
        assert_eq!(t.character_at(ByteOffset(3)), Ok(Some(0xe9)));
        assert_eq!(t.character_of_char(CharOffset(1)), Ok(Some(0x3fffe9)));
        assert_eq!(t.character_of_char(CharOffset(2)), Ok(Some(0xe9)));
        assert_eq!(t.byte_of_char(CharOffset(3)), Ok(ByteOffset(5)));
        assert_eq!(&*t.internal(ByteOffset(1)..ByteOffset(3)).unwrap(), &[0xc1, 0xa9]);
        assert_eq!(&*t.internal(ByteOffset(3)..ByteOffset(5)).unwrap(), &[0xc3, 0xa9]);
        // Searches tell the raw byte and the scalar of its code apart.
        assert_eq!(t.find_character(0x3fffe9, ByteOffset(0), ByteOffset(6)), Some(ByteOffset(1)));
        assert_eq!(t.find_character(0xe9, ByteOffset(0), ByteOffset(6)), Some(ByteOffset(3)));
        assert_eq!(t.find_character(0xe9, ByteOffset(0), ByteOffset(3)), None);
        // Edits before a raw byte move its offset; deleting it drops it.
        t.replace(ByteOffset(0)..ByteOffset(0), "xyz".as_bytes()).unwrap();
        assert!(t.is_raw_at(4) && !t.is_raw_at(1));
        assert_eq!(t.bytes(), &[b'x', b'y', b'z', b'a', 0xc1, 0xa9, 0xc3, 0xa9, b'b']);
        t.replace(ByteOffset(4)..ByteOffset(6), &[0xc0, 0x80]).unwrap();
        assert_eq!(t.character_at(ByteOffset(4)), Ok(Some(0x3fff80)));
        t.replace(ByteOffset(3)..ByteOffset(6), b"").unwrap();
        assert!(!t.has_raw());
        assert_eq!(t.as_str(), "xyzéb");
        // Overwriting swaps the listing both ways (subst-char-in-region).
        t.overwrite(ByteOffset(3)..ByteOffset(5), &[0xc1, 0xa9]).unwrap();
        assert_eq!(t.character_at(ByteOffset(3)), Ok(Some(0x3fffe9)));
        t.overwrite(ByteOffset(3)..ByteOffset(5), "é".as_bytes()).unwrap();
        assert!(!t.has_raw());
        // A raw byte at the tail fast path's seam and across chunks.
        let mut big = crate::text::Text::new("a".repeat(5000).as_bytes()).unwrap();
        big.replace(ByteOffset(2500)..ByteOffset(2500), &[0xc1, 0xbf]).unwrap();
        big.point = ByteOffset(big.len_bytes());
        assert!(big.append_utf8("tail"));
        assert_eq!(big.character_at(ByteOffset(2500)), Ok(Some(0x3fffff)));
        assert_eq!(big.bytes()[2500..2502], [0xc1, 0xbf]);
        assert_eq!(big.len_chars(), 5005);
    }

    #[test]
    fn raw_bytes_convert_with_unibyte_storage() {
        let mut t = crate::text::Text::new(&[b'A', 0xc1, 0xa9, b'B']).unwrap();
        t.point = ByteOffset(3);
        t.narrow(ByteOffset(1)..ByteOffset(4)).unwrap();
        t.put_property(ByteOffset(3)..ByteOffset(4), "p", Value::TRUE).unwrap();
        let binary = t.binary();
        assert_eq!(binary.bytes, [b'A', 0xe9, b'B']);
        assert_eq!(binary.point, ByteOffset(2));
        assert_eq!(binary.restriction, ByteOffset(1)..ByteOffset(3));
        assert_eq!(binary.properties.keys().copied().collect::<Vec<_>>(), [ByteOffset(2)]);
        let (back, grown) = crate::text::Text::decode_binary(&binary).unwrap();
        assert_eq!(grown, [1]);
        assert_eq!(back.bytes(), &[b'A', 0xc1, 0xa9, b'B']);
        assert_eq!(back.point, ByteOffset(3));
        assert_eq!(back.accessible(), ByteOffset(1)..ByteOffset(4));
        assert_eq!(back.property(ByteOffset(3), "p"), Ok(Some(Value::TRUE)));
        // str_as_multibyte: a valid sequence stays a character.
        let (out, grown) = octets_as_multibyte(&[0xe9, b'x', 0xc3, 0xa9, 0xff]);
        assert_eq!(out, [0xc1, 0xa9, b'x', 0xc3, 0xa9, 0xc1, 0xbf]);
        assert_eq!(grown, [0, 4]);
    }

    #[test]
    fn append_fast_path_updates_all_directory_boundaries() {
        let mut t = Text::new(b"").unwrap();
        let mut expected = String::new();
        for i in 0..3000 {
            let block = if i % 3 == 0 {
                "abcdefghij\n"
            } else {
                "aλ中🙂é\n"
            };
            assert!(t.append_chars(&block.chars().collect::<Vec<_>>()));
            expected.push_str(block);
        }
        for (c, (b, _)) in expected.char_indices().enumerate() {
            assert_eq!(t.byte_of_char(c), Ok(b));
            assert_eq!(t.char_of_byte(b), Ok(c));
        }
        assert_eq!(t.bytes(), expected.as_bytes());
        assert_eq!(t.point, ByteOffset(t.len_bytes()));
        assert_eq!(t.accessible(), 0..t.len_chars());
    }
    #[test]
    fn ascii_uses_the_same_directory_and_character_storage() {
        let t = Text::new(b"abc").unwrap();
        assert_eq!(t.bytes(), b"abc", "canonical UTF-8 storage");
        assert_eq!(t.rope.len_bytes(), 3);
    }
    #[test]
    fn width_changing_replace_updates_every_coordinate() {
        let mut t = Text::new("aλβЖ🙂z".as_bytes()).unwrap();
        assert_eq!(t.byte_of_char(2), Ok(3));
        t.replace(0..1, "🙂".as_bytes()).unwrap();
        assert_eq!(
            t.byte_of_char(2),
            Ok(6),
            "coordinate after width-changing replacement"
        );
    }
    #[test]
    fn boundaries() {
        contract::unicode_boundaries(make)
    }
    #[test]
    fn edits() {
        contract::edits(make)
    }
    #[test]
    fn invalidation() {
        contract::edit_invalidation(make)
    }
    #[test]
    fn rejected() {
        contract::rejected_edits(make)
    }
    #[test]
    fn modes() {
        // Text has no reinterpretation operation. Invalid encodings are not text.
        for bytes in [
            &[0xff][..],
            &[0xed, 0xa0, 0x80],
            &[0xf4, 0x90, 0x80, 0x80],
        ] {
            assert!(Text::new(bytes).is_err(), "non-scalar encoding accepted");
        }
    }
    #[test]
    fn narrowing() {
        contract::narrowing(make)
    }
    #[test]
    fn semantics() {
        contract::unicode_editor_semantics(make)
    }
    #[test]
    fn read_cache_thrashing_tail_seams_and_edits_match_flat_text() {
        let mut expected = format!(
            "{}{}{}",
            "a".repeat(4000),
            "λ中🙂\n".repeat(4000),
            "z".repeat(4000)
        );
        let mut t = crate::text::Text::new(expected.as_bytes()).unwrap();
        let check = |t: &crate::text::Text, expected: &str| {
            let offsets: Vec<_> = expected.char_indices().collect();
            for i in 0..200 {
                for c in [i, offsets.len() - i - 1, (i * 131) % offsets.len()] {
                    let (b, ch) = offsets[c];
                    assert_eq!(t.character_of_char(CharOffset(c)), Ok(Some(u32::from(ch))));
                    assert_eq!(t.byte_of_char(CharOffset(c)), Ok(ByteOffset(b)));
                    assert_eq!(t.char_of_byte(ByteOffset(b)), Ok(CharOffset(c)));
                    assert_eq!(t.character_at(ByteOffset(b)), Ok(Some(ch as i64)));
                }
            }
            assert_eq!(t.chunks().collect::<String>(), expected);
            assert!(t.cold_chunks.borrow().len() <= 31);
            assert!(t.tail.len() <= 1024);
            for (line, b) in std::iter::once(0)
                .chain(expected.match_indices('\n').map(|(b, _)| b + 1))
                .enumerate()
            {
                assert_eq!(t.byte_of_line(line), Ok(ByteOffset(b)));
                assert_eq!(t.line_of_byte(ByteOffset(b)), Ok(line));
            }
        };
        check(&t, &expected);
        let copied = t.clone();
        t.replace(ByteOffset(1)..ByteOffset(2), "🙂".as_bytes())
            .unwrap();
        expected.replace_range(1..2, "🙂");
        check(&t, &expected);
        assert_ne!(copied.as_str(), t.as_str());
        t.point = ByteOffset(t.len_bytes());
        for _ in 0..300 {
            assert!(t.append_utf8("λ🙂\n"));
            expected.push_str("λ🙂\n");
        }
        check(&t, &expected);
        let end = t.len_bytes();
        t.replace(ByteOffset(end - 1)..ByteOffset(end), b"!")
            .unwrap();
        expected.replace_range(end - 1..end, "!");
        check(&t, &expected);
        assert_eq!(t.as_str(), expected);
        let mut empty = crate::text::Text::new(b"").unwrap();
        assert!(empty.append_utf8("🙂\n"));
        assert!(empty.storage_bytes(true) >= 1024 + empty.tail.capacity());
        assert_eq!(empty.byte_of_line(1), Ok(ByteOffset(5)));
        assert_eq!(
            empty.character_at(ByteOffset(1)),
            Err("index out of range".into())
        );
    }
    #[test]
    fn bounded_work() {
        contract::bounded_unicode_conversion_work(make)
    }
}
