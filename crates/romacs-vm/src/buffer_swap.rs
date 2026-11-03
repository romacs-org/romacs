//! buffer.c Fbuffer_swap_text: two buffers exchange their text.
//!
//! A buffer object is its text (with point, narrowing, markers, overlays,
//! properties and modification counts) and its identity (name, liveness,
//! per-buffer variables, tables and keymap). Unibyteness is the object's
//! kind, so it travels with the text, as GNU swaps
//! `enable_multibyte_characters`: the two objects trade places in the
//! arena and their identities go back.
use super::*;
use crate::data::Object;

/// What stays with a buffer when its text leaves.
struct Identity {
    name: Option<String>,
    live: bool,
    inhibit_buffer_hooks: bool,
    per_buffer: crate::forwarded::PerBuffer,
    case_tables: [Value; 4],
    local_keymap: Value,
    syntax_table: Value,
    category_table: Value,
}

fn take_identity(object: &mut Object) -> Identity {
    macro_rules! take {
        ($b:expr) => {{
            let state = &mut $b.state;
            state.defun_start = None;
            Identity {
                name: state.name.take(),
                live: state.live,
                inhibit_buffer_hooks: state.inhibit_buffer_hooks,
                per_buffer: std::mem::take(&mut $b.per_buffer),
                case_tables: $b.case_tables,
                local_keymap: $b.local_keymap,
                syntax_table: $b.syntax_table,
                category_table: $b.category_table,
            }
        }};
    }
    match object {
        Object::Buffer(b) => take!(b),
        Object::ByteBuffer(b) => take!(b),
        _ => unreachable!("a buffer"),
    }
}

fn put_identity(object: &mut Object, id: Identity) {
    macro_rules! put {
        ($b:expr) => {{
            $b.state.name = id.name;
            $b.state.live = id.live;
            $b.state.inhibit_buffer_hooks = id.inhibit_buffer_hooks;
            $b.per_buffer = id.per_buffer;
            $b.case_tables = id.case_tables;
            $b.local_keymap = id.local_keymap;
            $b.syntax_table = id.syntax_table;
            $b.category_table = id.category_table;
        }};
    }
    match object {
        Object::Buffer(b) => put!(b),
        Object::ByteBuffer(b) => put!(b),
        _ => unreachable!("a buffer"),
    }
}

fn per_buffer(object: &mut Object) -> &mut crate::forwarded::PerBuffer {
    match object {
        Object::Buffer(b) => &mut b.per_buffer,
        Object::ByteBuffer(b) => &mut b.per_buffer,
        _ => unreachable!("a buffer"),
    }
}

fn state(object: &mut Object) -> &mut bytes::BufferState {
    match object {
        Object::Buffer(b) => &mut b.state,
        Object::ByteBuffer(b) => &mut b.state,
        _ => unreachable!("a buffer"),
    }
}

/// The per-buffer variables GNU swaps with the text.
const TEXT_VARIABLES: &[&str] = &[
    "buffer-undo-list", "mark-active", "bidi-display-reordering",
    "bidi-paragraph-direction", "bidi-paragraph-separate-re", "bidi-paragraph-start-re",
];

impl Vm {
    /// Swap the current buffer's text with OTHER's (Fbuffer_swap_text).
    pub(crate) fn buffer_swap_text(&mut self, other: Value) -> std::result::Result<(), prim::Fault> {
        use prim::Fault;
        let current = self.slots[self.buffer_slot].value;
        if !matches!(self.object_if(other), Some(Object::Buffer(_) | Object::ByteBuffer(_))) {
            return Err(Fault::Wrong("bufferp", other));
        }
        if !self.live_buffer_value(other) { return Err(Fault::Error("Cannot swap a dead buffer’s text".into())); }
        for buffer in [other, current] {
            match self.object_if(buffer) {
                Some(Object::Buffer(b)) if b.base != Value::NIL => return Err(Fault::Error("Cannot swap indirect buffers’s text".into())),
                Some(Object::Buffer(b)) if !b.indirect.is_empty() => return Err(Fault::Error("One of the buffers to swap has indirect buffers".into())),
                // VM2-IFC-HELD1: insert-file-contents holds its text buffer.
                Some(Object::ByteBuffer(b)) if b.held.is_some() =>
                    return Err(Fault::Error("VM2 cannot swap text while insert-file-contents decides its coding".into())),
                _ => {}
            }
        }
        let (ia, ib) = (current.index(0).map_err(Fault::Error)? - 1, other.index(0).map_err(Fault::Error)? - 1);
        let mut members = self.buffer_referrers(ia);
        members.extend(self.buffer_referrers(ib));
        members.sort_unstable();
        members.dedup();
        if ia != ib {
            self.objects.swap(ia, ib);
            let (a, b) = (take_identity(&mut self.objects[ia]), take_identity(&mut self.objects[ib]));
            put_identity(&mut self.objects[ia], b);
            put_identity(&mut self.objects[ib], a);
            // The identities came back; the text's variables go with it.
            let slots: Vec<u16> = TEXT_VARIABLES.iter().filter_map(|name| match self.forwarded.lookup(name) {
                Some(crate::forwarded::Fwd::Buffer(i)) => Some(i),
                _ => None,
            }).collect();
            let mut b_vars = std::mem::take(per_buffer(&mut self.objects[ib]));
            per_buffer(&mut self.objects[ia]).swap_text_slots(&mut b_vars, &slots);
            *per_buffer(&mut self.objects[ib]) = b_vars;
            // The markers and overlays of each text now belong to the other.
            let swapped = |b: Value| if b == current { Some(other) } else if b == other { Some(current) } else { None };
            for i in members {
                let to = match &mut self.objects[i] {
                    Object::Marker(m) => swapped(m.buffer).inspect(|&to| m.buffer = to),
                    Object::Overlay(o) => swapped(o.buffer).inspect(|&to| o.buffer = to),
                    _ => None,
                };
                if let Some(to) = to { self.note_referrer(to, i); }
            }
        }
        for index in [ia, ib] { state(&mut self.objects[index]).record_change(true); }
        // A window's markers stay in its own buffer: point at BEGV and the
        // start at 1, as set-window-buffer starts (VM2-WB1).
        let windows: Vec<Value> = self.referrers().windows.clone().into_iter().filter_map(|i| match &self.objects[i] {
            Object::Window(w) if !w.dead && (w.buffer == current || w.buffer == other) => Some(Value::reference(i + 1, 0)),
            _ => None,
        }).collect();
        for window in windows {
            let (buffer, point, start, id) = { let w = self.window(window).map_err(Fault::Error)?; (w.buffer, w.point, w.start, w.id) };
            // Its text changed with no edit its snapshot's key could see.
            self.display_state.forget_window(id);
            let begv = Value::integer(self.buffer_begv(buffer).map_err(Fault::Error)?).map_err(Fault::Error)?;
            self.set_marker_object(point, begv, buffer).map_err(Fault::Error)?;
            self.set_marker_object(start, Value::integer(1).map_err(Fault::Error)?, buffer).map_err(Fault::Error)?;
        }
        self.display_state.version += 1;
        Ok(())
    }

    /// BUFFER's accessible start as a character position.
    fn buffer_begv(&self, buffer: Value) -> Result<i64> {
        match self.object(buffer)? {
            Object::Buffer(b) => Ok(b.char_of_byte(b.accessible().start)?.0 as i64 + 1),
            Object::ByteBuffer(b) => Ok(b.restriction.start.0 as i64 + 1),
            _ => Err("buffer required".into()),
        }
    }
}
