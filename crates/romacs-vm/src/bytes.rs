//! Binary edge storage is not text and never validates or decodes UTF-8.
use crate::{Result, Value, coordinates::ByteOffset};
use std::{collections::BTreeMap, ops::Range};
/// Non-Lisp buffer identity and modification state; contains no moving Values.
#[derive(Clone, Debug)]
pub(crate) struct BufferState {
    pub(crate) name: Option<String>,
    pub(crate) live: bool,
    pub(crate) inhibit_buffer_hooks: bool,
    pub(crate) modification: crate::modification::Modification,
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
    fn deref(&self) -> &Self::Target { &self.modification }
}
impl std::ops::DerefMut for BufferState {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.modification }
}
#[derive(Clone)]
pub(crate) struct ByteBuffer {
    pub(crate) state: BufferState,
    pub bytes: Vec<u8>,
    pub syntax_table: Value,
    /// The buffer's category table; nil means the standard one (category.c).
    pub category_table: Value,
    pub(crate) case_tables: [Value; 4],
    pub point: ByteOffset,
    pub restriction: Range<ByteOffset>,
    /// Restrictions saved by save-restriction, by id. A unibyte buffer holds
    /// no markers, so it keeps each saved BEGV and ZV itself and moves them as
    /// a left- and a right-biased marker (editfns.c save_restriction_save_1).
    pub(crate) saved_restrictions: Vec<(u32, Range<ByteOffset>)>,
    /// Points saved by save-excursion, by id, each moved as GNU's
    /// left-biased excursion marker would move (editfns.c save_excursion_save).
    pub(crate) saved_points: Vec<(u32, ByteOffset)>,
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
romacs_snapshot::snapshot_struct!(ByteBuffer { state, bytes, syntax_table, category_table, case_tables, point, restriction, saved_restrictions, saved_points, properties, had_intervals, per_buffer, held });
/// insdel.c adjust_markers_for_{insert,delete,replace}: where a marker at P
/// lands when R is replaced by INSERTED bytes. RIGHT is its insertion type,
/// which matters only for a pure insertion at P.
fn moved(p: ByteOffset, r: &Range<ByteOffset>, inserted: usize, right: bool) -> ByteOffset {
    if p < r.start || (p == r.start && !(right && r.is_empty())) {
        p
    } else if p >= r.end {
        ByteOffset(p.0 - (r.end.0 - r.start.0) + inserted)
    } else {
        r.start
    }
}
impl ByteBuffer {
    pub fn replace(&mut self, r: Range<ByteOffset>, inserted: &[u8]) -> Result<()> {
        if r.start > r.end || r.end.0 > self.bytes.len() {
            return Err("index out of range".into());
        }
        if r.is_empty() && inserted.is_empty() {
            return Ok(());
        }
        let changed_bytes = (r.end.0 - r.start.0)
            .checked_add(inserted.len())
            .ok_or("buffer too large")?;
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
        for (_, saved) in &mut self.saved_restrictions {
            *saved = moved(saved.start, &r, inserted.len(), false)..moved(saved.end, &r, inserted.len(), true);
        }
        for (_, saved) in &mut self.saved_points {
            *saved = moved(*saved, &r, inserted.len(), false);
        }
        self.bytes
            .splice(r.start.0..r.end.0, inserted.iter().copied());
        if r.start != r.end || !inserted.is_empty() {
            self.state.record_text_change(changed_bytes);
        }
        Ok(())
    }
    /// insert_before_markers: INSERTED at point, after the saved points
    /// and bounds there, as adjust_markers_for_insert moves every marker at
    /// point when BEFORE_MARKERS.
    pub fn insert_before_markers(&mut self, inserted: &[u8]) -> Result<()> {
        let at = self.point;
        self.replace(at..at, inserted)?;
        let end = ByteOffset(at.0 + inserted.len());
        for (_, saved) in &mut self.saved_restrictions {
            if saved.start == at {
                saved.start = end;
            }
        }
        for (_, saved) in &mut self.saved_points {
            if *saved == at {
                *saved = end;
            }
        }
        Ok(())
    }
    pub fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.syntax_table);
        visit(&mut self.category_table);
        for root in &mut self.case_tables { visit(root); }
        self.per_buffer.visit_values(visit);
        if let Some(held) = &mut self.held { held.visit_values(visit); }
        for props in self.properties.values_mut() {
            for v in props.values_mut() {
                visit(v);
            }
        }
    }
    pub fn storage_bytes(&self, capacity: bool) -> usize {
        self.state.storage_bytes(capacity) + (if capacity {
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
