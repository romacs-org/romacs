//! Resumable keymap/chartab walkers. Native handles retain precisely the active
//! GNU subtable, even if a callback replaces its former parent's slot.
use super::*;
use crate::char_table::{
    CharTableNode, CharTableNodeHandle, CharTableSlot, visit_char_table_node_values,
};
use crate::prim::{Fault, Leaf};
use std::collections::HashSet;
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
#[derive(Clone)]
struct Frame {
    node: CharTableNodeHandle,
    top: Value,
    from: usize,
    to: i64,
    index: usize,
    phase: Phase,
}
impl Frame {
    fn new(node: CharTableNodeHandle, top: Value, from: usize, to: i64) -> Self {
        let n = node.lock().unwrap();
        let index = from.saturating_sub(n.min_char) / CharTableNode::WIDTHS[n.depth];
        drop(n);
        Self {
            node,
            top,
            from,
            to,
            index,
            phase: Phase::Read,
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
}
impl Chartab {
    fn new(cx: &Leaf<'_>, table: Value) -> std::result::Result<Self, Fault> {
        cx.map_table_validate(table)?;
        let root = cx.map_table_root(table);
        Ok(Self {
            table,
            val: CharTableNode::checked_get(&root, 0).map_err(Fault::Error)?,
            frames: vec![Frame::new(
                root,
                table,
                0,
                char_table::CharTable::LEN as i64 - 1,
            )],
            from: 0,
            to: char_table::CharTable::LEN as i64 - 1,
            final_emitted: false,
        })
    }
    fn parent(&mut self, cx: &Leaf<'_>, top: Value, from: usize, to: i64) -> std::result::Result<bool, Fault> {
        let parent = cx.map_table_parent(top);
        if parent == Value::NIL {
            return Ok(false);
        }
        cx.map_table_validate(parent)?;
        // GNU samples using the caller-local FROM, but recursion starts at
        // the shared range car, which a nested child may already have advanced.
        self.val = cx.map_table_raw_default(parent, from)?;
        self.frames
            .push(Frame::new(cx.map_table_root(parent), parent, self.from, to));
        Ok(true)
    }
    fn next(&mut self, cx: &mut Leaf<'_>) -> std::result::Result<Option<(Value, Value)>, Fault> {
        // Callbacks may mutate or detach retained nodes between resumptions.
        cx.map_table_validate(self.table)?;
        for frame in &self.frames {
            let (depth, min) = { let n = frame.node.lock().unwrap(); (n.depth, n.min_char) };
            CharTableNode::validate_traversal(&frame.node, depth, min).map_err(Fault::Error)?;
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
                return Ok((self.val != Value::NIL).then(|| (key(cx, self.from, self.to), self.val)));
            }
            let n = self.frames.len() - 1;
            let f = &self.frames[n];
            let (c, width, len) = {
                let node = f.node.lock().unwrap();
                let width = CharTableNode::WIDTHS[node.depth];
                (node.min_char + f.index * width, width, node.slots.len())
            };
            if f.index >= len || c as i64 > f.to {
                self.frames.pop();
                continue;
            }
            match f.phase.clone() {
                Phase::Child => {
                    self.advance(n);
                }
                Phase::Emitted(this) => {
                    self.transition(n, c, this);
                }
                Phase::Parent(this) => {
                    if self.val != Value::NIL && self.val != this {
                        self.to = c as i64 - 1;
                        self.frames[n].phase = Phase::Emitted(this);
                        return Ok(Some((key(cx, self.from, self.to), self.val)));
                    }
                    self.transition(n, c, this);
                }
                Phase::Read => {
                    // Snapshot THIS before any callback for the preceding run.
                    // Child identity is retained, not looked up again on resume.
                    let slot =
                        self.frames[n].node.lock().unwrap().slots[self.frames[n].index].clone();
                    match slot {
                        CharTableSlot::Node(node) => {
                            if self.frames[n].to >= (c + width) as i64 {
                                self.to = (c + width) as i64 - 1;
                            }
                            self.frames[n].phase = Phase::Child;
                            self.frames.push(Frame::new(
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
                                self.advance(n);
                                continue;
                            }
                            self.frames[n].phase = Phase::Parent(this);
                            if self.val == Value::NIL {
                                self.to = c as i64 - 1;
                                if self.parent(cx, self.frames[n].top, self.frames[n].from, self.to)?
                                {
                                    continue;
                                }
                            }
                            if self.val != Value::NIL {
                                self.to = c as i64 - 1;
                                self.frames[n].phase = Phase::Emitted(this);
                                return Ok(Some((key(cx, self.from, self.to), self.val)));
                            }
                            self.transition(n, c, this);
                        }
                    }
                }
            }
        }
    }
    fn advance(&mut self, n: usize) {
        self.to = self.frames[n].to;
        self.frames[n].index += 1;
        self.frames[n].phase = Phase::Read;
    }
    fn transition(&mut self, n: usize, c: usize, this: Value) {
        self.val = this;
        self.from = c;
        self.frames[n].from = c;
        self.advance(n);
    }
    fn visit(&mut self, seen: &mut HashSet<usize>, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.table);
        visit(&mut self.val);
        for f in &mut self.frames {
            visit(&mut f.top);
            match &mut f.phase {
                Phase::Parent(v) | Phase::Emitted(v) => visit(v),
                Phase::Read | Phase::Child => {}
            }
            visit_char_table_node_values(&f.node, seen, visit);
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
    pub(super) fn visit_values(
        &mut self,
        seen: &mut HashSet<usize>,
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
                    table.visit(seen, visit);
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
