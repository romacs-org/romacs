//! A snapshot of a busy evaluator's call stack: frames as bounded JSON,
//! taken at an engine safe point and read off the main VM (`host::inspect_*`).
//!
//! The reader runs no Lisp: it allocates no Lisp object, collects nothing,
//! calls nothing back and changes no VM state, so it is safe wherever the
//! engine polls, nested engines and `inhibit-quit' included. It never uses
//! the printers, which walk whole objects, keep print-number state and can
//! dispatch to Lisp. Work is bounded, not only output: a cons or vector is
//! walked within a node budget and cut with `elided', so a circular list
//! costs what a long one does. Redaction here needs no Lisp policy.
use super::*;
use crate::data::Object;

/// The frames kept, innermost first; the rest are counted.
const FRAMES: usize = 256;
/// The snapshot's size past which no more is added.
const BYTES: usize = 64 * 1024;
/// How deep a value is walked.
const DEPTH: usize = 3;
/// The arguments shown per frame; the rest are counted.
const ARGS: usize = 16;
/// The nodes walked per argument.
const NODES: usize = 64;
/// The characters of a string shown.
const STRING_CHARS: usize = 80;
/// A string longer than this cannot be scanned within budget, so it is
/// redacted whole.
const SCAN_BYTES: usize = 4096;
/// A name or string holding one of these is sensitive, in any case.
const MARKERS: [&str; 5] = ["auth-source", "password", "passwd", "secret", "token"];

fn sensitive(text: &[u8]) -> bool {
    MARKERS.iter().any(|marker| text.windows(marker.len()).any(|window| window.eq_ignore_ascii_case(marker.as_bytes())))
}

fn json_string(out: &mut String, text: &str) {
    out.push_str(&serde_json::Value::from(text).to_string());
}

/// A name, cut to what is shown: nothing past the cut is scanned or copied.
fn name(out: &mut String, text: &str) {
    let shown: String = text.chars().take(STRING_CHARS).collect();
    json_string(out, &shown);
    if shown.len() < text.len() { out.push_str(r#","cut":true"#); }
}

/// A private string's bytes, with no walk of them. An image string gives
/// none: reading one verifies and validates its whole body.
fn string_bytes(vm: &Vm, value: Value) -> Option<&[u8]> {
    match vm.object_if(value)? {
        Object::String(s) | Object::RawString(s) => Some(s.view().bytes),
        Object::Bytes(bytes) => Some(bytes),
        _ => None,
    }
}

fn string(out: &mut String, bytes: &[u8]) {
    if bytes.len() > SCAN_BYTES || sensitive(bytes) {
        out.push_str(&format!(r#"{{"len":{},"redacted":true}}"#, bytes.len()));
        return;
    }
    let text = String::from_utf8_lossy(bytes);
    let shown: String = text.chars().take(STRING_CHARS).collect();
    out.push_str(r#"{"str":"#);
    json_string(out, &shown);
    out.push_str(&format!(r#","len":{}}}"#, bytes.len()));
}

fn object_kind(object: &Object) -> &'static str {
    match object {
        Object::Buffer(_) | Object::ByteBuffer(_) => "buffer",
        Object::Marker(_) => "marker",
        Object::Overlay(_) => "overlay",
        Object::Window(_) => "window",
        Object::Frame(_) => "frame",
        Object::Process(_) => "process",
        Object::Bignum(_) => "bignum",
        Object::CharTable(_) => "char-table",
        _ => "object",
    }
}

impl Vm {
    fn inspect_symbol(&self, value: Value) -> Option<&str> {
        self.symbol_names.get(value.index_if(7)?.checked_sub(1)?).map(String::as_str)
    }
    fn inspect_items(&self, items: &[Value], depth: usize, nodes: &mut usize, out: &mut String) {
        out.push_str(r#"{"vector":["#);
        let mut shown = 0;
        for &item in items {
            if *nodes == 0 { break; }
            if shown > 0 { out.push(','); }
            self.inspect_value(item, depth - 1, nodes, out);
            shown += 1;
        }
        out.push_str(&format!(r#"],"len":{}}}"#, items.len()));
    }
    fn inspect_list(&self, list: Value, depth: usize, nodes: &mut usize, out: &mut String) {
        out.push_str(r#"{"list":["#);
        let (mut tail, mut first) = (list, true);
        while *nodes > 0 && let Ok((car, cdr)) = self.cons_pair(tail) {
            if !first { out.push(','); }
            first = false;
            self.inspect_value(car, depth - 1, nodes, out);
            tail = cdr;
        }
        out.push(']');
        if tail.tag() == 3 {
            out.push_str(r#","elided":true"#);
        } else if tail != Value::NIL {
            out.push_str(r#","tail":"#);
            self.inspect_value(tail, 1, nodes, out);
        }
        out.push('}');
    }
    fn inspect_value(&self, value: Value, depth: usize, nodes: &mut usize, out: &mut String) {
        // Past the byte limit the walk is over: a container's loop ends on
        // the node count, and a circular list would never end otherwise.
        if out.len() > BYTES { *nodes = 0; }
        if *nodes == 0 { return out.push_str(r#"{"elided":true}"#); }
        *nodes -= 1;
        if value == Value::NIL { return out.push_str("null"); }
        if value == Value::TRUE { return out.push_str("true"); }
        if value == Value::UNBOUND { return out.push_str(r#"{"type":"unbound"}"#); }
        if let Some(n) = value.integer_value() { return out.push_str(&n.to_string()); }
        match value.tag() {
            7 => match self.inspect_symbol(value) {
                Some(symbol) => { out.push_str(r#"{"sym":"#); name(out, symbol); out.push('}'); }
                None => out.push_str(r#"{"type":"symbol"}"#),
            },
            3 if depth == 0 => out.push_str(r#"{"type":"cons"}"#),
            3 => self.inspect_list(value, depth, nodes, out),
            0 => self.inspect_object(value, depth, nodes, out),
            4 => out.push_str(r#"{"type":"function"}"#),
            6 => out.push_str(r#"{"type":"primitive"}"#),
            _ => out.push_str(r#"{"type":"other"}"#),
        }
    }
    fn inspect_object(&self, value: Value, depth: usize, nodes: &mut usize, out: &mut String) {
        if Vm::image_string(value) { return out.push_str(r#"{"type":"string"}"#); }
        if let Some(bytes) = string_bytes(self, value) { return string(out, bytes); }
        match self.object_if(value) {
            Some(Object::Float(f)) if f.is_finite() => out.push_str(&f.to_string()),
            Some(Object::Vector(items) | Object::Record(items)) if depth > 0 => self.inspect_items(items, depth, nodes, out),
            Some(object) => out.push_str(&format!(r#"{{"type":"{}"}}"#, object_kind(object))),
            None => out.push_str(r#"{"type":"unknown"}"#),
        }
    }
    fn inspect_frame(&self, index: usize, out: &mut String) {
        let function = self.backtrace.frames[index].function;
        let symbol = self.inspect_symbol(function);
        out.push_str(r#"{"fn":"#);
        match symbol {
            Some(symbol) => name(out, symbol),
            None => self.inspect_value(function, 0, &mut 1, out),
        }
        let args = self.backtrace_arguments_if(index).unwrap_or(&[]);
        out.push_str(&format!(r#","argc":{}"#, args.len()));
        // Past the scan budget a name counts as sensitive, as a string does.
        if symbol.is_some_and(|symbol| symbol.len() > SCAN_BYTES || sensitive(symbol.as_bytes())) {
            out.push_str(r#","args":"redacted"}"#);
            return;
        }
        out.push_str(r#","args":["#);
        for (at, &arg) in args.iter().take(ARGS).enumerate() {
            if out.len() > BYTES { break; }
            if at > 0 { out.push(','); }
            self.inspect_value(arg, DEPTH, &mut { NODES }, out);
        }
        out.push_str("]}");
    }
    /// The call stack as JSON, innermost frame first.
    pub(crate) fn inspect_snapshot(&self) -> String {
        let total = self.backtrace.frames.len();
        let mut out = String::from(r#"{"frames":["#);
        let mut shown = 0;
        for index in (0..total).rev().take(FRAMES) {
            if out.len() > BYTES { break; }
            if shown > 0 { out.push(','); }
            self.inspect_frame(index, &mut out);
            shown += 1;
        }
        out.push_str(&format!(r#"],"depth":{total},"shown":{shown}"#));
        let buffer = self.slots[self.buffer_slot].value;
        if let Some(name) = self.buffer_state(buffer).ok().and_then(|state| state.name.as_deref()) {
            out.push_str(r#","buffer":"#);
            string(&mut out, name.as_bytes());
        }
        out.push('}');
        out
    }
    /// Answer the pending inspect request. Out of line, so the poll that
    /// calls it keeps its sibling-call dispatch.
    #[cold]
    #[inline(never)]
    pub(crate) fn inspect_capture(&self) {
        let generation = crate::host::inspect_generation();
        crate::host::inspect_publish(generation, self.inspect_snapshot());
    }
}
