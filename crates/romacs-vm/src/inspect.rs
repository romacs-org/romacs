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

/// The frames by name, for a poll inside a native loop that holds other
/// parts of the VM: only what never changes while it runs, with no values.
pub(crate) fn snapshot_names(backtrace: &backtrace::Backtrace, symbols: &[String], place: &str) -> String {
    let frames = &backtrace.frames;
    let mut out = String::from(r#"{"frames":["#);
    for (at, frame) in frames.iter().rev().take(FRAMES).enumerate() {
        if at > 0 { out.push(','); }
        let symbol = frame.function.index_if(7).and_then(|n| n.checked_sub(1)).and_then(|n| symbols.get(n));
        out.push_str(r#"{"fn":"#);
        match symbol {
            Some(symbol) => name(&mut out, symbol),
            None => out.push_str(r#"{"type":"function"}"#),
        }
        out.push_str(r#","args":"unavailable: in native"}"#);
    }
    out.push_str(&format!(r#"],"depth":{},"in":"#, frames.len()));
    json_string(&mut out, place);
    out.push('}');
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Redisplay reaches the display iterator without `quittable_motion`,
    /// through the route the host's layout takes. A press waits for it, and
    /// an inspection is answered inside it. The same press ends a motion
    /// Lisp asked for: the iterator does poll here.
    #[test]
    fn redisplay_keeps_a_quit_pending_and_answers_inspection() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let signal = std::sync::Arc::new(QuitSignal::new());
        vm.share_quit_signal(signal.clone());
        vm.enter_busy();
        vm.eval_source("(progn (insert (make-string 20000 ?x)) (goto-char 1))").unwrap();
        let window = vm.selected_window_value().unwrap();
        assert!(signal.interrupt(), "a busy evaluator takes the press");
        crate::host::inspect_request();
        let shown = crate::prim::window_shown_elements(&mut vm, window, 1, 20001);
        assert_eq!(shown.map(|(elements, _)| elements.len()).ok(), Some(20000));
        assert!(vm.quit.due(), "redisplay took the quit");
        assert!(!crate::host::inspect_pending(), "redisplay left the inspection unanswered");
        vm.motion_quit = true;
        assert!(crate::prim::window_shown_elements(&mut vm, window, 1, 20001).is_err(), "a Lisp-called motion did not quit");
    }

    /// A display span's end is found past every other run inside it: the walk
    /// polls, for a motion that wants a quit and for an inspection.
    #[test]
    fn a_property_walk_over_many_runs_polls() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let signal = std::sync::Arc::new(QuitSignal::new());
        vm.share_quit_signal(signal.clone());
        vm.enter_busy();
        vm.eval_source("(progn (insert (make-string 8000 ?x)) (put-text-property 1 8000 'display \"d\")
          (let ((i 1)) (while (< i 8000) (put-text-property i (1+ i) 'face (if (= 0 (% i 2)) 'bold 'italic)) (setq i (1+ i)))))").unwrap();
        let (display, buffer) = (vm.intern("display"), vm.slots[vm.buffer_slot].value);
        let start = Value::integer(1).unwrap();
        crate::host::inspect_request();
        assert!(vm.next_single_char_property_change_values(start, display, buffer, Value::NIL).is_ok());
        assert!(!crate::host::inspect_pending(), "the walk left the inspection unanswered");
        assert!(signal.interrupt(), "a busy evaluator takes the press");
        vm.motion_quit = true;
        assert!(vm.next_single_char_property_change_values(start, display, buffer, Value::NIL).is_err(), "the walk did not quit");
    }
}
