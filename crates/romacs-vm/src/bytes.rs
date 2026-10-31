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
}
romacs_snapshot::snapshot_struct!(BufferState { name, live, inhibit_buffer_hooks, modification, defun_start });
impl Default for BufferState {
    fn default() -> Self {
        Self { name: None, live: true, inhibit_buffer_hooks: false, modification: Default::default(), defun_start: None }
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
}
romacs_snapshot::snapshot_struct!(ByteBuffer { state, bytes, syntax_table, category_table, case_tables, local_keymap, point, restriction, anchors, properties, had_intervals, per_buffer, held });
impl ByteBuffer {
    /// An empty buffer's storage, with STATE.
    pub(crate) fn empty(state: BufferState, per_buffer: crate::forwarded::PerBuffer) -> Self {
        ByteBuffer {
            state, bytes: vec![], syntax_table: Value::NIL, category_table: Value::NIL, case_tables: [Value::NIL; 4], local_keymap: Value::NIL,
            point: ByteOffset(0), restriction: ByteOffset(0)..ByteOffset(0), anchors: AnchorTree::new(0),
            properties: Default::default(), had_intervals: false, per_buffer, held: None,
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
