//! Resumable keymap/chartab walkers. Native handles retain precisely the active
//! GNU subtable, even if a callback replaces its former parent's slot.
use super::*;
use crate::char_table::{
    CharTableNode, CharTableNodeHandle, CharTableSlot, collector_visit_node,
};
use crate::prim::{Fault, Leaf};
#[cfg(test)]
#[path = "map_keymap_gc_tests.rs"]
mod gc_tests;

#[derive(Clone)]
enum Phase {
    Read,
    Child,
    Parent(Value),
    Emitted(Value),
}
romacs_snapshot::snapshot_enum!(Phase {
    Phase::Read => (),
    Phase::Child => (),
    Phase::Parent(f0) => (f0),
    Phase::Emitted(f0) => (f0),
});
#[derive(Clone)]
struct Frame {
    node: CharTableNodeHandle,
    top: Value,
    from: usize,
    to: i64,
    index: usize,
    phase: Phase,
    decoder: bool,
}
romacs_snapshot::snapshot_struct!(Frame { node, top, from, to, index, phase, decoder });
impl Frame {
    fn new(node: CharTableNodeHandle, top: Value, from: usize, to: i64) -> Self {
        let n = node.lock().unwrap();
        let index = from.saturating_sub(n.min_char()) / CharTableNode::WIDTHS[n.depth()];
        drop(n);
        Self {
            node,
            top,
            from,
            to,
            index,
            phase: Phase::Read,
            decoder: false,
        }
    }
}
#[derive(Clone)]
struct Chartab {
    table: Value,
    frames: Vec<Frame>,
    val: Value,
    from: usize,
    to: i64,
    final_emitted: bool,
    range: Option<Value>,
    final_decoder: bool,
}
romacs_snapshot::snapshot_struct!(Chartab { table, frames, val, from, to, final_emitted, range, final_decoder });
impl Chartab {
    fn new(cx: &mut Leaf<'_>, table: Value) -> std::result::Result<Self, Fault> {
        cx.map_table_validate(table)?;
        let root = cx.map_table_root(table);
        Ok(Self {
            table,
            val: cx.map_table_first(table)?,
            frames: vec![Frame::new(
                root,
                table,
                0,
                char_table::CharTable::LEN as i64 - 1,
            )],
            from: 0,
            to: char_table::CharTable::LEN as i64 - 1,
            final_emitted: false,
            range: None,
            final_decoder: false,
        })
    }
    fn parent(
        &mut self,
        cx: &mut Leaf<'_>,
        top: Value,
        from: usize,
        to: i64,
    ) -> std::result::Result<bool, Fault> {
        let parent = cx.map_table_parent(top);
        if parent == Value::NIL {
            return Ok(false);
        }
        cx.map_table_validate(parent)?;
        // GNU samples using the caller-local FROM, but recursion starts at
        // the shared range car, which a nested child may already have advanced.
        self.val = cx.map_table_raw_default(parent, from)?;
        self.frames
            .push(self.frame(cx, cx.map_table_root(parent), parent, self.from, to));
        Ok(true)
    }
    fn next(&mut self, cx: &mut Leaf<'_>) -> std::result::Result<Option<(Value, Value)>, Fault> {
        // Callbacks may mutate the table between resumptions. A node keeps its
        // geometry for life and was checked when the walk entered it, so only
        // the slot counts the index arithmetic relies on are rechecked.
        for frame in &self.frames {
            let n = frame.node.lock().unwrap();
            if n.depth() > 3 || n.slots().len() != CharTableNode::SIZES[n.depth()] {
                return Err(Fault::Error("invalid sub-char-table geometry".into()));
            }
        }
        loop {
            if self.frames.is_empty() {
                if self.final_emitted {
                    return Ok(None);
                }
                let table = self.table;
                if self.val == Value::NIL && self.parent(cx, table, self.from, self.to)? {
                    self.table = cx.map_table_parent(table);
                    continue;
                }
                self.final_emitted = true;
                self.sync_range(cx);
                return if self.val == Value::NIL {
                    Ok(None)
                } else {
                    Ok(Some(self.binding(cx, self.table, self.final_decoder)?))
                };
            }
            let n = self.frames.len() - 1;
            let f = &self.frames[n];
            let (c, width, len) = {
                let node = f.node.lock().unwrap();
                let width = CharTableNode::WIDTHS[node.depth()];
                (node.min_char() + f.index * width, width, node.slots().len())
            };
            if f.index >= len || c as i64 > f.to {
                self.frames.pop();
                continue;
            }
            match f.phase.clone() {
                Phase::Child => {
                    self.advance(cx, n);
                }
                Phase::Emitted(this) => {
                    self.transition(cx, n, c, this);
                }
                Phase::Parent(this) => {
                    if self.val != Value::NIL && self.val != this {
                        self.to = c as i64 - 1;
                        self.frames[n].phase = Phase::Emitted(this);
                        return Ok(Some(self.binding(
                            cx,
                            self.frames[n].top,
                            self.frames[n].decoder,
                        )?));
                    }
                    self.transition(cx, n, c, this);
                }
                Phase::Read => {
                    // Snapshot THIS before any callback for the preceding run.
                    // Child identity is retained, not looked up again on resume.
                    let slot = cx.map_table_slot(
                        self.frames[n].top,
                        &self.frames[n].node,
                        self.frames[n].index,
                    )?;
                    match slot {
                        CharTableSlot::Node(node) => {
                            let depth = self.frames[n].node.lock().unwrap().depth() + 1;
                            CharTableNode::validate_node(&node, depth, c).map_err(Fault::Error)?;
                            if self.frames[n].to >= (c + width) as i64 {
                                self.to = (c + width) as i64 - 1;
                            }
                            self.frames[n].phase = Phase::Child;
                            self.frames.push(self.frame(
                                cx,
                                node,
                                self.frames[n].top,
                                self.from,
                                self.to,
                            ));
                        }
                        CharTableSlot::Value(mut this) => {
                            if this == Value::NIL {
                                this = cx.map_table_default(self.frames[n].top);
                            }
                            if this == self.val {
                                self.advance(cx, n);
                                continue;
                            }
                            self.frames[n].phase = Phase::Parent(this);
                            if self.val == Value::NIL {
                                self.to = c as i64 - 1;
                                if self.parent(
                                    cx,
                                    self.frames[n].top,
                                    self.frames[n].from,
                                    self.to,
                                )? {
                                    continue;
                                }
                            }
                            if self.val != Value::NIL {
                                self.to = c as i64 - 1;
                                self.frames[n].phase = Phase::Emitted(this);
                                return Ok(Some(self.binding(
                                    cx,
                                    self.frames[n].top,
                                    self.frames[n].decoder,
                                )?));
                            }
                            self.transition(cx, n, c, this);
                        }
                    }
                }
            }
        }
    }
    fn frame(
        &self,
        cx: &Leaf<'_>,
        node: CharTableNodeHandle,
        top: Value,
        from: usize,
        to: i64,
    ) -> Frame {
        let mut frame = Frame::new(node, top, from, to);
        frame.decoder = self.range.is_some() && cx.map_table_has_decoder(top);
        frame
    }
    fn binding(
        &self,
        cx: &mut Leaf<'_>,
        top: Value,
        decoder: bool,
    ) -> std::result::Result<(Value, Value), Fault> {
        let value = if decoder {
            cx.map_table_decode(top, self.val)?
        } else {
            self.val
        };
        Ok((self.key(cx), value))
    }
    fn sync_range(&self, cx: &mut Leaf<'_>) {
        if let Some(range) = self.range {
            cx.setcar(range, Value::integer(self.from as i64).unwrap())
                .unwrap_or_else(|_| unreachable!("live traversal range"));
            cx.setcdr(range, Value::integer(self.to).unwrap())
                .unwrap_or_else(|_| unreachable!("live traversal range"));
        }
    }
    fn key(&self, cx: &mut Leaf<'_>) -> Value {
        self.sync_range(cx);
        if self.from as i64 == self.to {
            Value::integer(self.to).unwrap()
        } else {
            self.range.unwrap_or_else(|| key(cx, self.from, self.to))
        }
    }
    fn advance(&mut self, cx: &mut Leaf<'_>, n: usize) {
        self.to = self.frames[n].to;
        self.frames[n].index += 1;
        self.frames[n].phase = Phase::Read;
        self.sync_range(cx);
    }
    fn transition(&mut self, cx: &mut Leaf<'_>, n: usize, c: usize, this: Value) {
        self.val = this;
        self.from = c;
        self.frames[n].from = c;
        self.advance(cx, n);
    }
    fn visit(&mut self, seen: &mut HashSet<usize>, since: u64, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.table);
        visit(&mut self.val);
        if let Some(range) = &mut self.range {
            visit(range);
        }
        for f in &mut self.frames {
            visit(&mut f.top);
            match &mut f.phase {
                Phase::Parent(v) | Phase::Emitted(v) => visit(v),
                Phase::Read | Phase::Child => {}
            }
            collector_visit_node(&f.node, seen, since, visit);
        }
    }
}
fn key(cx: &mut Leaf<'_>, from: usize, to: i64) -> Value {
    let from = Value::integer(from as i64).unwrap();
    let to = Value::integer(to as i64).unwrap();
    if from == to { from } else { cx.cons(from, to) }
}
#[derive(Clone)]
enum MapFrame {
    Resolve {
        object: Value,
        required: bool,
    },
    Outer(Value),
    Embedded(Value),
    Tail(Value),
    Advance(Value),
    Vector {
        tail: Value,
        vector: Value,
        index: usize,
        len: usize,
    },
    Table {
        tail: Value,
        table: Chartab,
    },
}
romacs_snapshot::snapshot_enum!(MapFrame {
    MapFrame::Resolve { object, required } => (object, required),
    MapFrame::Outer(f0) => (f0),
    MapFrame::Embedded(f0) => (f0),
    MapFrame::Tail(f0) => (f0),
    MapFrame::Advance(f0) => (f0),
    MapFrame::Vector { tail, vector, index, len } => (tail, vector, index, len),
    MapFrame::Table { tail, table } => (tail, table),
});
pub(super) enum Advance {
    Binding(Value, Value),
    Load(Value, Value),
    Done(Value),
}
#[derive(Clone)]
pub(super) struct Cursor {
    stack: Vec<MapFrame>,
    internal: bool,
    result: Value,
}
romacs_snapshot::snapshot_struct!(Cursor { stack, internal, result });
impl Cursor {
    pub(super) fn new(map: Value, internal: bool) -> Self {
        Self {
            stack: vec![MapFrame::Resolve {
                object: map,
                required: true,
            }],
            internal,
            result: Value::NIL,
        }
    }
    pub(super) fn char_table(cx: &mut Leaf<'_>, value: Value) -> std::result::Result<Self, Fault> {
        let mut table = Chartab::new(cx, value)?;
        table.final_decoder = cx.map_table_has_decoder(value);
        table.frames[0].decoder = table.final_decoder;
        table.range = Some(cx.cons(
            Value::integer(0).unwrap(),
            Value::integer(char_table::CharTable::LEN as i64 - 1).unwrap(),
        ));
        Ok(Self {
            stack: vec![MapFrame::Table {
                tail: Value::NIL,
                table,
            }],
            internal: false,
            result: Value::NIL,
        })
    }
    pub(super) fn next(&mut self, cx: &mut Leaf<'_>) -> std::result::Result<Advance, Fault> {
        while let Some(frame) = self.stack.pop() {
            match frame {
                MapFrame::Resolve { object, required } => match cx.map_resolve(object, true)? {
                    Some((definition, true)) => {
                        self.stack.push(MapFrame::Resolve { object, required });
                        return Ok(Advance::Load(definition, object));
                    }
                    Some((map, false)) => {
                        if self.internal {
                            self.stack.push(MapFrame::Tail(cx.pair(map).unwrap().1));
                        } else {
                            self.stack.push(MapFrame::Outer(map));
                        }
                    }
                    None if required => return Err(Fault::Wrong("keymapp", object)),
                    None => {}
                },
                MapFrame::Outer(map) => {
                    let Some((car, _)) = cx.pair(map) else {
                        continue;
                    };
                    if cx.map_resolve(car, false)?.is_some() {
                        self.stack.push(MapFrame::Embedded(map));
                        self.stack.push(MapFrame::Resolve {
                            object: car,
                            required: true,
                        });
                    } else {
                        let tail = if cx.map_is_head(car) {
                            cx.pair(map).unwrap().1
                        } else {
                            map
                        };
                        self.stack.push(MapFrame::Tail(tail));
                    }
                }
                MapFrame::Embedded(map) => {
                    let tail = cx.pair(map).map_or(Value::NIL, |p| p.1);
                    if cx.pair(tail).is_some() {
                        self.stack.push(MapFrame::Outer(tail));
                    } else {
                        self.stack.push(MapFrame::Resolve {
                            object: tail,
                            required: false,
                        });
                    }
                }
                MapFrame::Advance(tail) => {
                    self.stack
                        .push(MapFrame::Tail(cx.pair(tail).map_or(Value::NIL, |p| p.1)));
                }
                MapFrame::Tail(tail) => {
                    let binding = cx.pair(tail).map(|p| p.0);
                    if binding.is_none()
                        || if let Some(v) = binding {
                            cx.map_is_head(v) || cx.map_resolve(v, false)?.is_some()
                        } else {
                            false
                        }
                    {
                        if self.internal {
                            self.result = tail;
                        } else if binding.is_some() {
                            self.stack.push(MapFrame::Outer(tail));
                        } else {
                            self.stack.push(MapFrame::Resolve {
                                object: tail,
                                required: false,
                            });
                        }
                        continue;
                    }
                    let binding = binding.unwrap();
                    if let Some(pair) = cx.pair(binding) {
                        self.stack.push(MapFrame::Advance(tail));
                        return Ok(Advance::Binding(pair.0, pair.1));
                    }
                    match cx.kind(binding) {
                        crate::prim::Kind::Vector => {
                            self.stack.push(MapFrame::Vector {
                                tail,
                                vector: binding,
                                index: 0,
                                len: cx.vector_len(binding)?,
                            });
                        }
                        crate::prim::Kind::CharTable => {
                            self.stack.push(MapFrame::Table {
                                tail,
                                table: Chartab::new(cx, binding)?,
                            });
                        }
                        _ => self.stack.push(MapFrame::Advance(tail)),
                    }
                }
                MapFrame::Vector {
                    tail,
                    vector,
                    index,
                    len,
                } => {
                    if index == len {
                        self.stack.push(MapFrame::Advance(tail));
                        continue;
                    }
                    self.stack.push(MapFrame::Vector {
                        tail,
                        vector,
                        index: index + 1,
                        len,
                    });
                    let k = Value::integer(index as i64).unwrap();
                    return Ok(Advance::Binding(k, cx.vector_get(vector, k)?));
                }
                MapFrame::Table { tail, mut table } => {
                    if let Some(pair) = table.next(cx)? {
                        self.stack.push(MapFrame::Table { tail, table });
                        return Ok(Advance::Binding(pair.0, pair.1));
                    }
                    self.stack.push(MapFrame::Advance(tail));
                }
            }
        }
        Ok(Advance::Done(self.result))
    }
    /// Leave unwalked the autoloaded keymap `next` just asked to load, as
    /// `map_keymap` without AUTOLOAD does: whether the walk requires it (an
    /// embedded keymap, which `get_keymap` then refuses) or may end there (a
    /// parent).
    pub(super) fn decline_load(&mut self) -> bool {
        matches!(self.stack.pop(), Some(MapFrame::Resolve { required: true, .. }))
    }
    pub(super) fn collector_visit_values(
        &mut self,
        seen: &mut HashSet<usize>,
        since: u64,
        visit: &mut impl FnMut(&mut Value),
    ) {
        visit(&mut self.result);
        for f in &mut self.stack {
            match f {
                MapFrame::Resolve { object: v, .. }
                | MapFrame::Outer(v)
                | MapFrame::Embedded(v)
                | MapFrame::Tail(v)
                | MapFrame::Advance(v) => visit(v),
                MapFrame::Vector { tail, vector, .. } => {
                    visit(tail);
                    visit(vector);
                }
                MapFrame::Table { tail, table } => {
                    visit(tail);
                    table.visit(seen, since, visit);
                }
            }
        }
    }
    pub(super) fn storage_bytes(&self, capacity: bool, seen: &mut HashSet<usize>) -> usize {
        size_of::<Self>()
            + (if capacity {
                self.stack.capacity()
            } else {
                self.stack.len()
            }) * size_of::<MapFrame>()
            + self
                .stack
                .iter()
                .map(|f| {
                    if let MapFrame::Table { table, .. } = f {
                        (if capacity {
                            table.frames.capacity()
                        } else {
                            table.frames.len()
                        }) * size_of::<Frame>()
                            + table
                                .frames
                                .iter()
                                .map(|f| CharTableNode::storage_bytes(&f.node, capacity, seen))
                                .sum::<usize>()
                    } else {
                        0
                    }
                })
                .sum::<usize>()
    }
}
