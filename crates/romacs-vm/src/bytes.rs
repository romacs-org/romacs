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
}
impl Default for BufferState {
    fn default() -> Self {
        Self { name: None, live: true, inhibit_buffer_hooks: false, modification: Default::default() }
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
    pub(crate) case_tables: [Value; 4],
    pub point: ByteOffset,
    pub restriction: Range<ByteOffset>,
    pub properties: BTreeMap<ByteOffset, BTreeMap<String, Value>>,
    /// GNU's per-buffer variable slots (`struct buffer`, forwarded.rs).
    pub per_buffer: crate::forwarded::PerBuffer,
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
        self.bytes
            .splice(r.start.0..r.end.0, inserted.iter().copied());
        if r.start != r.end || !inserted.is_empty() {
            self.state.record_text_change(changed_bytes);
        }
        Ok(())
    }
    pub fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.syntax_table);
        for root in &mut self.case_tables { visit(root); }
        self.per_buffer.visit_values(visit);
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
