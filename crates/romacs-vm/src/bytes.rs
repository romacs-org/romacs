//! Binary edge storage is not text and never validates or decodes UTF-8.
use crate::{Result, Value, anchors::AnchorTree, coordinates::ByteOffset};
use std::{collections::BTreeMap, ops::Range};
/// Non-Lisp buffer identity and modification state; contains no moving Values.
#[derive(Clone, Debug)]
pub(crate) struct BufferState {
    pub(crate) name: Option<String>,
    pub(crate) live: bool,
    pub(crate) inhibit_buffer_hooks: bool,
    /// Shared with the buffer's indirect buffers, as GNU's MODIFF and
    /// SAVE_MODIFF are in their shared text.
    pub(crate) modification: crate::indirect::Shared<crate::modification::Modification>,
    /// syntax.c's find_start_* cache, tagged with `Vm::defun_start_epoch`
    /// when this buffer wrote it: GNU keeps one cache for the last buffer.
    pub(crate) defun_start: Option<(u64, crate::syntax_scan::DefunStart)>,
    /// The terminal session this buffer shows, where it shows one.
    pub(crate) terminal: Option<u64>,
}
romacs_snapshot::snapshot_struct!(BufferState { name, live, inhibit_buffer_hooks, modification, defun_start, terminal });
impl Default for BufferState {
    fn default() -> Self {
        Self { name: None, live: true, inhibit_buffer_hooks: false, modification: Default::default(), defun_start: None, terminal: None }
    }
}
impl BufferState {
    pub(crate) fn modified(&self) -> bool { self.modiff > self.save_modiff }
    pub(crate) fn record_change(&mut self, chars: bool) {
        self.modification.changed(1, chars);
    }
    pub(crate) fn record_text_change(&mut self, len: usize) {
        if len == 0 { return; }
        self.modification.changed(len, true);
    }
    pub(crate) fn kill(&mut self) { self.live = false; self.name = None; }
    pub(crate) fn storage_bytes(&self, capacity: bool) -> usize {
        self.name.as_ref().map_or(0, |n| if capacity { n.capacity() } else { n.len() })
    }
}
impl std::ops::Deref for BufferState {
    type Target = crate::modification::Modification;
    fn deref(&self) -> &Self::Target { &*self.modification }
}
impl std::ops::DerefMut for BufferState {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut *self.modification }
}
#[derive(Clone)]
pub(crate) struct ByteBuffer {
    pub(crate) state: BufferState,
    pub bytes: Vec<u8>,
    pub syntax_table: Value,
    /// The buffer's category table; nil means the standard one (category.c).
    pub category_table: Value,
    pub(crate) case_tables: [Value; 4],
    /// The buffer's local keymap (`use-local-map'), as a text buffer's.
    pub local_keymap: Value,
    pub point: ByteOffset,
    pub restriction: Range<ByteOffset>,
    /// The markers in this buffer, at byte offsets, which here are its
    /// characters (insdel.c adjust_markers_for_*); as a text buffer's.
    pub anchors: AnchorTree,
    pub properties: BTreeMap<ByteOffset, BTreeMap<String, Value>>,
    /// GNU keeps a buffer's interval tree once it had a property.
    pub had_intervals: bool,
    /// GNU's per-buffer variable slots (`struct buffer`, forwarded.rs).
    pub per_buffer: crate::forwarded::PerBuffer,
    /// The empty multibyte buffer, with its markers and overlays, while
    /// insert-file-contents holds it unibyte to decide the coding
    /// (fileio.c decide_coding_unwind); its anchors all stay at BEG.
    pub(crate) held: Option<Box<crate::annotations::AnnotatedBuffer>>,
    /// What a window shows of these bytes (`shown'), once one has.
    pub(crate) mirror: Option<Box<Mirror>>,
}
romacs_snapshot::snapshot_struct!(ByteBuffer { state, bytes, syntax_table, category_table, case_tables, local_keymap, point, restriction, anchors, properties, had_intervals, per_buffer, held } at_rest { mirror });

/// The text of a unibyte buffer as display reads it: each byte one
/// character, so its positions are the buffer's, a byte above 127 the
/// raw-byte character GNU shows as an escape. Every edit of the bytes edits
/// it, and it holds the buffer's overlays.
#[derive(Clone)]
pub(crate) struct Mirror {
    text: crate::annotations::AnnotatedBuffer,
}
impl Mirror {
    /// The byte range of the mirror's text that R, of the buffer's bytes, is.
    fn range(&self, r: &Range<ByteOffset>) -> Result<Range<ByteOffset>> {
        let byte = |at: ByteOffset| self.text.byte_of_char(crate::coordinates::CharOffset(at.0));
        Ok(byte(r.start)?..byte(r.end)?)
    }
    /// As the bytes' `replace_with_bias': a replacement inserts before the
    /// markers at its end and then deletes (adjust_markers_for_replace).
    fn replace(&mut self, r: &Range<ByteOffset>, inserted: &[u8], before_markers: bool) -> Result<()> {
        let range = self.range(r)?;
        let text = crate::text::octets_to_multibyte(inserted);
        if range.is_empty() { return self.text.replace_with_bias(range, &text, before_markers); }
        self.text.replace_with_bias(range.end..range.end, &text, true)?;
        self.text.replace_with_bias(range, b"", false)
    }
}
/// A unibyte buffer's or insertion's text properties, by byte.
pub(crate) type ByteProperties = BTreeMap<ByteOffset, BTreeMap<String, Value>>;

impl ByteBuffer {
    /// Bring the mirror to the bytes, narrowing and point now.
    pub(crate) fn mirror_now(&mut self) -> Result<()> {
        if self.mirror.is_none() {
            let text = crate::text::Text::new(&crate::text::octets_to_multibyte(&self.bytes))?;
            self.mirror = Some(Box::new(Mirror { text: text.into() }));
        }
        let (restriction, point, state) = (self.restriction.clone(), self.point, self.state.clone());
        let Some(mirror) = &mut self.mirror else { unreachable!("just built") };
        mirror.text.state = state;
        let byte = |text: &crate::annotations::AnnotatedBuffer, at: ByteOffset| text.byte_of_char(crate::coordinates::CharOffset(at.0));
        let view = (byte(&mirror.text, restriction.start)?..byte(&mirror.text, restriction.end)?, byte(&mirror.text, point)?);
        mirror.text.set_view(view.0, view.1);
        Ok(())
    }
    /// The first byte after AT, at most END, whose properties differ from
    /// AT's (`next_change' of a text buffer's).
    pub(crate) fn next_property_change(&self, at: ByteOffset, end: ByteOffset) -> ByteOffset {
        if at >= end { return end; }
        let here = self.properties.get(&at);
        let mut expected = at.0 + 1;
        for (&p, values) in self.properties.range(ByteOffset(at.0 + 1)..end) {
            if p.0 != expected && here.is_some() { return ByteOffset(expected); }
            if Some(values) != here { return p; }
            expected = p.0 + 1;
        }
        if here.is_some() { ByteOffset(expected).min(end) } else { end }
    }
    /// The last byte before AT, at least START, where the properties of the
    /// byte before it differ from those of the byte before AT.
    pub(crate) fn previous_property_change(&self, at: ByteOffset, start: ByteOffset) -> ByteOffset {
        if at <= start { return start; }
        let here = self.properties.get(&ByteOffset(at.0 - 1));
        let mut expected = at.0 - 1;
        for (&p, values) in self.properties.range(start..ByteOffset(at.0 - 1)).rev() {
            if p.0 + 1 != expected && here.is_some() { return ByteOffset(expected); }
            if Some(values) != here { return ByteOffset(p.0 + 1); }
            expected = p.0;
        }
        if here.is_some() { ByteOffset(expected).max(start) } else { start }
    }
    /// The mirror, once a window has shown these bytes.
    pub(crate) fn shown(&self) -> Option<&crate::annotations::AnnotatedBuffer> {
        self.mirror.as_ref().map(|m| &m.text)
    }
    pub(crate) fn shown_mut(&mut self) -> Option<&mut crate::annotations::AnnotatedBuffer> {
        self.mirror.as_mut().map(|m| &mut m.text)
    }
    /// The byte at AT is now BYTE: no marker moves.
    pub(crate) fn set_byte(&mut self, at: usize, byte: u8) -> Result<()> {
        self.bytes[at] = byte;
        let Some(mirror) = &mut self.mirror else { return Ok(()) };
        let range = mirror.range(&(ByteOffset(at)..ByteOffset(at + 1)))?;
        mirror.text.rewrite_character(range, &crate::text::octets_to_multibyte(&[byte]))
    }
    /// BYTES become the empty buffer's text, every marker staying at BEG.
    pub(crate) fn fill(&mut self, bytes: &[u8]) -> Result<()> {
        debug_assert!(self.bytes.is_empty());
        self.bytes = bytes.to_vec();
        self.point = ByteOffset(0);
        self.restriction = ByteOffset(0)..ByteOffset(bytes.len());
        match &mut self.mirror {
            Some(mirror) => mirror.replace(&(ByteOffset(0)..ByteOffset(0)), bytes, false),
            None => Ok(()),
        }
    }
    /// PROPERTIES, by offset from AT, onto the bytes just inserted there.
    pub(crate) fn graft_properties(&mut self, at: ByteOffset, properties: ByteProperties) {
        self.had_intervals |= !properties.is_empty();
        self.properties.extend(properties.into_iter().map(|(p, values)| (ByteOffset(at.0 + p.0), values)));
    }
    /// make_buffer_string_both with properties for R: its bytes, as a
    /// unibyte string holding R's text properties when it has any.
    pub(crate) fn text_object(&self, r: Range<ByteOffset>) -> crate::data::Object {
        let bytes = self.bytes[r.start.0..r.end.0].to_vec();
        let properties: BTreeMap<ByteOffset, BTreeMap<String, Value>> = self.properties.range(r.clone())
            .filter(|(_, values)| !values.is_empty())
            .map(|(&at, values)| (ByteOffset(at.0 - r.start.0), values.clone())).collect();
        if properties.is_empty() { return crate::data::unibyte_object(bytes); }
        let mut string = crate::data::LispString::from_bytes(bytes, false);
        string.properties = crate::properties::Properties::from_map(&properties);
        crate::data::string_object(string)
    }
    /// An empty buffer's storage, with STATE.
    pub(crate) fn empty(state: BufferState, per_buffer: crate::forwarded::PerBuffer) -> Self {
        ByteBuffer {
            state, bytes: vec![], syntax_table: Value::NIL, category_table: Value::NIL, case_tables: [Value::NIL; 4], local_keymap: Value::NIL,
            point: ByteOffset(0), restriction: ByteOffset(0)..ByteOffset(0), anchors: AnchorTree::new(0),
            properties: Default::default(), had_intervals: false, per_buffer, held: None, mirror: None,
        }
    }
    pub fn replace(&mut self, r: Range<ByteOffset>, inserted: &[u8]) -> Result<()> {
        self.replace_with_bias(r, inserted, false)
    }
    /// Replace R with INSERTED; with BEFORE_MARKERS an insertion leaves
    /// every marker at it after the new bytes, as insert-before-markers does.
    fn replace_with_bias(&mut self, r: Range<ByteOffset>, inserted: &[u8], before_markers: bool) -> Result<()> {
        if r.start > r.end || r.end.0 > self.bytes.len() {
            return Err("index out of range".into());
        }
        if r.is_empty() && inserted.is_empty() {
            return Ok(());
        }
        let changed_bytes = (r.end.0 - r.start.0)
            .checked_add(inserted.len())
            .ok_or("buffer too large")?;
        let (a, b) = (u32::try_from(r.start.0).map_err(|_| "buffer too large")?, u32::try_from(r.end.0).map_err(|_| "buffer too large")?);
        let len = u32::try_from(inserted.len()).map_err(|_| "buffer too large")?;
        self.anchors.extent().checked_sub(b - a).and_then(|n| n.checked_add(len)).ok_or("buffer too large")?;
        let shift = |p: ByteOffset| {
            ByteOffset(if p < r.start {
                p.0
            } else if p <= r.end {
                r.start.0 + inserted.len()
            } else {
                p.0 - (r.end.0 - r.start.0) + inserted.len()
            })
        };
        self.properties = std::mem::take(&mut self.properties)
            .into_iter()
            .filter_map(|(p, v)| {
                if r.contains(&p) {
                    None
                } else {
                    Some((
                        if p >= r.end {
                            ByteOffset(p.0 - (r.end.0 - r.start.0) + inserted.len())
                        } else {
                            p
                        },
                        v,
                    ))
                }
            })
            .collect();
        self.point = shift(self.point);
        let old = self.restriction.clone();
        self.restriction = (if old.start <= r.start {
            old.start
        } else {
            shift(old.start)
        })..shift(old.end);
        self.bytes
            .splice(r.start.0..r.end.0, inserted.iter().copied());
        if let Some(mirror) = &mut self.mirror { mirror.replace(&r, inserted, before_markers)?; }
        // adjust_markers_for_insert for an insertion; for a replacement,
        // adjust_markers_for_replace: a marker at its end follows the new
        // text and one inside it goes to its start, as inserting before the
        // markers at its end and then deleting it moves them.
        if a == b {
            assert!(self.anchors.insert(a, len, before_markers));
        } else {
            assert!(self.anchors.insert(b, len, true));
            assert!(self.anchors.delete(a, b));
        }
        if r.start != r.end || !inserted.is_empty() {
            self.state.record_text_change(changed_bytes);
        }
        Ok(())
    }
    /// insert_before_markers: INSERTED at point, after every marker there
    /// (adjust_markers_for_insert with BEFORE_MARKERS).
    pub fn insert_before_markers(&mut self, inserted: &[u8]) -> Result<()> {
        let at = self.point;
        self.replace_with_bias(at..at, inserted, true)
    }
    pub fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.syntax_table);
        visit(&mut self.category_table);
        for root in &mut self.case_tables { visit(root); }
        visit(&mut self.local_keymap);
        self.per_buffer.visit_values(visit);
        if let Some(held) = &mut self.held { held.visit_values(visit); }
        if let Some(mirror) = &mut self.mirror { mirror.text.visit_values(visit); }
        for props in self.properties.values_mut() {
            for v in props.values_mut() {
                visit(v);
            }
        }
    }
    pub fn storage_bytes(&self, capacity: bool) -> usize {
        self.state.storage_bytes(capacity) + self.anchors.storage_bytes(capacity) + (if capacity {
            self.bytes.capacity()
        } else {
            self.bytes.len()
        }) + self
            .properties
            .values()
            .map(|p| {
                size_of::<ByteOffset>()
                    + size_of::<BTreeMap<String, Value>>()
                    + p.keys()
                        .map(|k| size_of::<String>() + size_of::<Value>() + k.capacity())
                        .sum::<usize>()
            })
            .sum::<usize>()
    }
}
