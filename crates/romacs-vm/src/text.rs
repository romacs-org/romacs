//! UTF-8 rope storage; properties and restrictions remain byte-canonical.
//! The cached ASCII property changes conversion cost, never the payload type.
use crate::properties::Properties;
use crate::{Result, Value, coordinates::*};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Default)]
struct ChunkCursor {
    text: String,
    byte: usize,
    scalar: usize,
    chars: usize,
    middle_byte: usize,
    middle_char: usize,
}
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
#[derive(Clone)]
pub struct Text {
    rope: ropey_stable::Rope,
    bytes_len: usize,
    chars_len: usize,
    tail: String,
    tail_chars: usize,
    flat: std::sync::OnceLock<String>,
    is_ascii: bool,
    coordinate: std::cell::Cell<(usize, usize)>,
    chunk: std::cell::RefCell<ChunkCursor>,
    cold_chunks: std::cell::RefCell<std::collections::BTreeMap<usize, ChunkCursor>>,
    cold_order: std::cell::RefCell<std::collections::VecDeque<usize>>,
    #[cfg(test)]
    scanned_bytes: std::cell::Cell<usize>,
    restriction: Range<ByteOffset>,
    pub(crate) properties: Properties,
    pub(crate) state: crate::bytes::BufferState,
    pub(crate) point: ByteOffset,
    pub(crate) syntax_table: Value,
    /// The buffer's category table; nil means the standard one (category.c).
    pub(crate) category_table: Value,
    #[cfg(test)]
    coordinate_steps: std::cell::Cell<usize>,
    #[cfg(test)]
    property_edit_visits: usize,
}
impl Text {
    pub fn new(bytes: &[u8]) -> Result<Self> {
        let s = std::str::from_utf8(bytes).map_err(|_| "invalid Unicode text")?;
        let rope = ropey_stable::Rope::from_str(s);
        let chars_len = rope.len_chars();
        Ok(Self {
            rope,
            bytes_len: s.len(),
            chars_len,
            tail: String::new(),
            tail_chars: 0,
            flat: std::sync::OnceLock::new(),
            is_ascii: s.is_ascii(),
            coordinate: std::cell::Cell::new((0, 0)),
            chunk: std::cell::RefCell::new(ChunkCursor::default()),
            cold_chunks: std::cell::RefCell::new(std::collections::BTreeMap::new()),
            cold_order: std::cell::RefCell::new(std::collections::VecDeque::new()),
            #[cfg(test)]
            scanned_bytes: std::cell::Cell::new(0),
            restriction: ByteOffset(0)..ByteOffset(s.len()),
            properties: Properties::new(),
            state: Default::default(),
            point: ByteOffset(0),
            syntax_table: Value::NIL,
            category_table: Value::NIL,
            #[cfg(test)]
            coordinate_steps: std::cell::Cell::new(0),
            #[cfg(test)]
            property_edit_visits: 0,
        })
    }
    pub fn as_str(&self) -> &str {
        self.flat.get_or_init(|| {
            let mut text = self.rope.to_string();
            text.push_str(&self.tail);
            text
        })
    }
    pub(crate) fn binary(&self) -> crate::bytes::ByteBuffer {
        crate::bytes::ByteBuffer {
            state: self.state.clone(),
            bytes: self.as_str().as_bytes().to_vec(),
            syntax_table: self.syntax_table,
            category_table: self.category_table,
            case_tables: [Value::NIL; 4],
            point: self.point,
            restriction: self.restriction.clone(),
            saved_restrictions: Vec::new(),
            saved_points: Vec::new(),
            properties: self
                .properties
                .to_map()
                .into_iter()
                .filter(|(at, _)| self.boundary(*at).is_ok())
                .collect(),
            had_intervals: self.properties.has_intervals(),
            per_buffer: Default::default(),
            held: None,
        }
    }
    pub(crate) fn decode_binary(b: &crate::bytes::ByteBuffer) -> Result<Self> {
        let mut text = Self::new(&b.bytes)?;
        text.boundary(b.point)?;
        text.validate_range(&b.restriction)?;
        for &p in b.properties.keys() {
            text.boundary(p)?;
        }
        text.point = b.point;
        text.state = b.state.clone();
        text.syntax_table = b.syntax_table;
        text.category_table = b.category_table;
        text.restriction = b.restriction.clone();
        text.properties = Properties::from_map(&b.properties);
        for (&at, values) in &b.properties {
            if at.0 < b.bytes.len() {
                let width = text.as_str()[at.0..].chars().next().unwrap().len_utf8();
                for (key, value) in values {
                    text.properties
                        .put(at..ByteOffset(at.0 + width), key, *value);
                }
            }
        }
        Ok(text)
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
    pub fn bytes(&self) -> &[u8] {
        self.as_str().as_bytes()
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
                .map(|s| s.chars().next().map(|c| c as i64))
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
        Ok(Some(ch as i64))
    }
    /// Scalar lookup keeps conversion and decoding in one chunk borrow.
    #[inline(always)]
    pub(crate) fn character_of_char(&self, c: CharOffset) -> Result<Option<char>> {
        if c.0 > self.chars_len {
            return Err("index out of range".into());
        }
        if c.0 == self.chars_len {
            return Ok(None);
        }
        if c.0 >= self.chars_len - self.tail_chars {
            let at = str_indices::chars::to_byte_idx(
                &self.tail,
                c.0 - (self.chars_len - self.tail_chars),
            );
            return Ok(self.tail[at..].chars().next());
        }
        let chunk = self.chunk_for_char(c.0);
        if chunk.chars == chunk.text.len() {
            return Ok(Some(chunk.text.as_bytes()[c.0 - chunk.scalar] as char));
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
        Ok(Some(ch))
    }
    pub fn slice(&self, r: Range<ByteOffset>) -> Result<&str> {
        self.as_str()
            .get(r.start.0..r.end.0)
            .ok_or_else(|| "index out of range".into())
    }
    pub(crate) fn find_character(
        &self,
        ch: u32,
        from: ByteOffset,
        to: ByteOffset,
    ) -> Option<ByteOffset> {
        let needle = char::from_u32(ch)?;
        let text = self.slice(from..to).ok()?;
        let at = text.find(needle)?;
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
        self.invalidate_reads();
        if self.tail.len() + s.len() > 1024 {
            self.flush_tail();
        }
        self.tail.push_str(s);
        self.tail_chars += chars;
        self.bytes_len += s.len();
        self.chars_len += chars;
        self.is_ascii &= s.is_ascii();
        self.point.0 += s.len();
        self.restriction.end.0 += s.len();
        self.state.record_text_change(chars);
        true
    }
    fn flush_tail(&mut self) {
        if !self.tail.is_empty() {
            self.rope
                .insert(self.chars_len - self.tail_chars, &self.tail);
            self.tail.clear();
            self.tail_chars = 0;
        }
    }
    pub fn replace(&mut self, r: Range<ByteOffset>, inserted: &[u8]) -> Result<()> {
        self.validate_range(&r)?;
        let s = std::str::from_utf8(inserted).map_err(|_| "invalid Unicode text")?;
        if r.is_empty() && s.is_empty() {
            return Ok(());
        }
        self.flush_tail();
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
        let property_work = self.properties.replace(r.clone(), s.len());
        #[cfg(test)]
        {
            self.property_edit_visits = property_work;
        }
        #[cfg(not(test))]
        let _ = property_work;
        self.point = shift(self.point);
        let old = self.restriction.clone();
        self.restriction = (if old.start <= r.start {
            old.start
        } else {
            shift(old.start)
        })..shift(old.end);
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
        self.state.record_text_change(end.0 - start.0 + s.chars().count());
        Ok(())
    }
    /// Overwrite the characters of R with as many characters of the same
    /// total byte length, keeping text properties, point and markers, as
    /// GNU's subst-char-in-region does. The caller records the change.
    pub(crate) fn overwrite(&mut self, r: Range<ByteOffset>, replacement: &str) -> Result<()> {
        self.validate_range(&r)?;
        if replacement.len() != r.end.0 - r.start.0 { return Err("overwrite length differs".into()); }
        self.flush_tail();
        let start = if self.is_ascii { r.start.0 } else { self.rope.byte_to_char(r.start.0) };
        let end = if self.is_ascii { r.end.0 } else { self.rope.byte_to_char(r.end.0) };
        self.invalidate_reads();
        self.rope.remove(start..end);
        self.rope.insert(start, replacement);
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
    /// Width of a single line's text. Tabs/control layout belongs to the renderer.
    pub fn display_width(&self, r: Range<ByteOffset>) -> Result<DisplayColumn> {
        Ok(DisplayColumn(UnicodeWidthStr::width(self.slice(r)?)))
    }
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.syntax_table);
        visit(&mut self.category_table);
        self.properties.visit_values(visit);
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
            + self.state.storage_bytes(capacity)
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
        let initial = text.state.modification;
        assert!(text.append_utf8(""));
        text.replace(ByteOffset(0)..ByteOffset(0), b"").unwrap();
        assert_eq!(text.state.modification, initial);
        assert!(text.replace(ByteOffset(0)..ByteOffset(0), &[0xff]).is_err());
        assert_eq!(text.state.modification, initial);
        assert!(text.append_utf8("λ"));
        assert_eq!(text.state.modification.modiff, 2);
        assert_eq!(text.state.modification.chars_modiff, 2);
        // Replacing identical bytes is still a delete/insert operation.
        text.replace(ByteOffset(0)..ByteOffset(2), "λ".as_bytes()).unwrap();
        assert_eq!(text.state.modification.modiff, 4);
        let before = text.state.modification;
        assert!(text.replace(ByteOffset(1)..ByteOffset(2), b"").is_err());
        assert_eq!(text.state.modification, before);
        let mut binary = text.binary();
        assert_eq!(binary.state.modification, before);
        binary.replace(ByteOffset(0)..ByteOffset(0), b"").unwrap();
        assert_eq!(binary.state.modification, before);
        binary.replace(ByteOffset(0)..ByteOffset(2), b"abcd").unwrap();
        assert_eq!(binary.state.modification.modiff, 7);
        assert_eq!(binary.state.modification.chars_modiff, 7);
        binary.state.modification.set_modified(false);
        binary.state.modification.autosave_modiff = 8;
        let decoded = crate::text::Text::decode_binary(&binary).unwrap();
        assert_eq!(decoded.state.modification, binary.state.modification);
        let wrapped: crate::annotations::AnnotatedBuffer = decoded.into();
        assert_eq!(wrapped.state.modification, binary.state.modification);
        assert_eq!(wrapped.clone().state.modification, binary.state.modification);
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
        fn slice(&self, r: Range<usize>) -> Result<&[u8]> {
            self.0
                .slice(
                    self.0.byte_of_char(CharOffset(r.start))?
                        ..self.0.byte_of_char(CharOffset(r.end))?,
                )
                .map(str::as_bytes)
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
        assert!(Text::new(&[0xc0, 0x80, 0xc1, 0xbf]).is_err());
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
            &[0xc0, 0x80],
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
                    assert_eq!(t.character_of_char(CharOffset(c)), Ok(Some(ch)));
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
