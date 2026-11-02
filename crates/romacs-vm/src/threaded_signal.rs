//! Pre-unwind signal search shared by ordinary and reentrant engines.
use super::*;

#[derive(Clone)]
pub(crate) enum SignalHandler {
    Condition(Value),
    Catch(Value),
    Bind { conditions: Value, function: Value, group: usize },
    Mask(usize),
    /// A compile-time macro expansion (`Vm::expanding_alone'): every signal
    /// it raises stops here and no catch beneath it is live, so no handler
    /// around the compilation sees an escape GNU would raise at the form's
    /// reach, inside the form's own handlers.
    Boundary,
}
romacs_snapshot::snapshot_enum!(SignalHandler {
    SignalHandler::Condition(f0) => (f0),
    SignalHandler::Catch(f0) => (f0),
    SignalHandler::Bind { conditions, function, group } => (conditions, function, group),
    SignalHandler::Mask(f0) => (f0),
    SignalHandler::Boundary => (),
});

impl SignalHandler {
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        match self {
            Self::Condition(value) | Self::Catch(value) => visit(value),
            Self::Bind { conditions, function, .. } => { visit(conditions); visit(function); }
            Self::Mask(_) | Self::Boundary => {}
        }
    }
}

fn member(vm: &Vm, wanted: Value, mut values: Value) -> bool {
    let mut seen = HashSet::default();
    while let Ok((value, tail)) = vm.cons_pair(values) {
        if value == wanted { return true; }
        if !seen.insert(values.0) { break; }
        values = tail;
    }
    false
}

fn matches(vm: &Vm, conditions: Value, mut choices: Value) -> bool {
    if choices.tag() != 3 { return choices == Value::TRUE || member(vm, choices, conditions); }
    let mut seen = HashSet::default();
    while let Ok((choice, tail)) = vm.cons_pair(choices) {
        if choice == Value::TRUE || member(vm, choice, conditions) { return true; }
        if !seen.insert(choices.0) { break; }
        choices = tail;
    }
    false
}

pub(super) fn release_control(e: &mut Engine<'_>, control: &Control) {
    let mark = match control.kind {
        ControlKind::HandlerBind { mark } => Some(mark),
        _ => control.registry,
    };
    if let Some(mark) = mark {
        debug_assert!(mark <= e.vm.signal_handlers.len());
        e.vm.signal_handlers.truncate(mark);
    }
}

/// A normalized error has already completed its non-unwinding search.  Carry
/// its selected condition target through eval/load and cleanup unchanged.
pub(super) fn prepare_signal(e: &mut Engine<'_>, hooked: bool, failed_depth: Option<usize>) -> bool {
    let mut replaced = false;
    if let Some(Escape::Signal(..)) = e.escape.as_ref() {
        // A new signal; an error carried through cleanup keeps its origin.
        e.vm.thrown_no_catch = false;
    }
    if !hooked && let Some(&Escape::Signal(condition, data)) = e.escape.as_ref()
        && !speculative(e, condition)
        && let Some(replacement) = signal_hook(e, condition, data, failed_depth)
    {
        e.escape = Some(replacement);
        replaced = true;
    }
    if let Some(Escape::Signal(condition, data)) = e.escape.as_ref() {
        let condition = *condition;
        let object = e.vm.list_value(&[condition]);
        let index = object.index(3).unwrap();
        e.vm.conses[index].1 = *data;
        let conditions = e.vm.signal_conditions(condition);
        e.escape = Some(Escape::Error { condition, object, conditions, target: None });
        replaced |= search(e);
        if !replaced && debugger_wanted(e)
            && let Some(Escape::Error { conditions, object, .. }) = e.escape.as_ref()
            && let Some(replacement) = call_debugger(e, *conditions, *object, failed_depth)
        {
            e.escape = Some(replacement);
            prepare_signal(e, false, None);
            replaced = true;
        }
    }
    replaced
}

/// Whether CONDITION's handler search ends at a compile-time expansion's
/// `Boundary': GNU signals it only at the form's reach, where
/// `signal-hook-function' hears of it, so it hears nothing now.
fn speculative(e: &mut Engine<'_>, condition: Value) -> bool {
    if !e.vm.signal_handlers.iter().any(|h| matches!(h, SignalHandler::Boundary)) {
        return false;
    }
    let conditions = e.vm.signal_conditions(condition);
    let mut at = e.vm.signal_handlers.len();
    while at != 0 {
        at -= 1;
        match e.vm.signal_handlers[at] {
            SignalHandler::Mask(skip) => at = skip,
            SignalHandler::Boundary => return true,
            SignalHandler::Condition(choices) if matches(e.vm, conditions, choices) => return false,
            _ => {}
        }
    }
    false
}

/// GNU signal_or_quit after its handler search: whether the debugger is
/// considered for the pending error, then maybe_call_debugger's tests that
/// need no Lisp. It is considered with `debug-on-signal', when no handler
/// catches the error, or when the catching clause lists `debug' (the
/// command loop's catch-all is `(debug t)', GNU's C handler `error').
/// `romacs--call-debugger-p' (rlisp/vm/prelude.el) makes the rest.
fn debugger_wanted(e: &mut Engine<'_>) -> bool {
    let Some(Escape::Error { conditions, target, .. }) = e.escape.as_ref() else { return false; };
    let (conditions, clause) = match target.and_then(|at| e.vm.signal_handlers.get(at)) {
        // A compile-time expansion's escape: GNU signals it at the form's
        // reach, where the debugger is considered then.
        Some(SignalHandler::Boundary) => return false,
        Some(SignalHandler::Condition(choices)) => (*conditions, *choices),
        _ => (*conditions, Value::NIL),
    };
    let set = |e: &mut Engine<'_>, name: &str| {
        let slot = e.vm.slot(name);
        !matches!(e.vm.variable_value(slot), Ok(Value::NIL | Value::UNBOUND) | Err(_))
    };
    let considered = clause == Value::NIL
        || (clause.tag() == 3 && { let debug = e.vm.intern("debug"); member(e.vm, debug, clause) })
        || set(e, "debug-on-signal");
    if !considered || set(e, "inhibit-debugger") { return false; }
    let quit = e.vm.intern("quit");
    let wanted = if member(e.vm, quit, conditions) { set(e, "debug-on-quit") } else { set(e, "debug-on-error") };
    let helper = e.vm.slot(DEBUGGER_HELPER);
    wanted && !matches!(e.vm.slots[helper].function, Value::NIL | Value::UNBOUND)
}

/// The Lisp half of maybe_call_debugger's tests.
pub(super) const DEBUGGER_HELPER: &str = "romacs--call-debugger-p";

fn search(e: &mut Engine<'_>) -> bool {
    let mut at = e.vm.signal_handlers.len();
    while at != 0 {
        at -= 1;
        let handler = e.vm.signal_handlers[at].clone();
        let Some(Escape::Error { condition, object, conditions, .. }) = e.escape.as_ref() else { break; };
        let (condition, conditions, object) = (*condition, *conditions, *object);
        match handler {
            SignalHandler::Mask(skip) => at = skip,
            SignalHandler::Boundary => {
                if let Some(Escape::Error { target, .. }) = e.escape.as_mut() { *target = Some(at); }
                break;
            }
            SignalHandler::Condition(choices) if matches(e.vm, conditions, choices) => {
                if let Some(Escape::Error { target, .. }) = e.escape.as_mut() { *target = Some(at); }
                break;
            }
            SignalHandler::Bind { conditions: choices, function, group } if matches(e.vm, conditions, choices) => {
                e.vm.publish_signal_conditions(condition, conditions);
                if handler_callback(e, function, object, group) { return true; }
            }
            _ => {}
        }
    }
    false
}

/// Publish the suspended engine's copies before reader_call permits collection.
/// The reader boundary owns register forwarding; this owner forwards controls
/// and inline caches in place without moving their instruction allocations.
fn handler_callback(e: &mut Engine<'_>, function: Value, object: Value, group: usize) -> bool {
    let mask = e.vm.signal_handlers.len();
    e.vm.signal_handlers.push(SignalHandler::Mask(group));
    let roots = e.suspend_code();
    let saved_depth = e.vm.eval_depth;
    e.vm.eval_depth = e.depth_base + e.frames.len();
    let token = e.vm.max_ensure_room(200);
    let result = e.vm.reader_call(function, &[object]);
    let replacement = e.vm.propagated_escape.take();
    let hooked = std::mem::take(&mut e.vm.propagated_escape_hooked);
    let depth = e.vm.propagated_eval_depth.take();
    e.resume_code(roots);
    e.fatal_image_failure |= e.vm.execution_fatal;
    e.code.resize_with(e.vm.functions.len(), Vec::new);
    let r = e.regs;
    if unsafe { widen_registers(e, r, e.vm.register_width) }.is_none() {
        e.error = Some("register storage exhausted".into());
        e.fatal_image_failure = true;
    }
    let escaped = match (result, replacement) {
        (_, Some(escape)) => { e.escape = Some(escape); true }
        (Err(error), None) => {
            let message = e.vm.make_lisp_string(error.into_bytes(), true);
            let data = e.vm.list_value(&[message]);
            e.escape = Some(Escape::Signal(e.vm.intern("error"), data));
            true
        }
        (Ok(_), None) => false,
    };
    if escaped { prepare_signal(e, hooked, depth); }
    if let Some(old) = token { e.vm.restore_stack_limits(old); }
    e.vm.eval_depth = saved_depth;
    e.vm.signal_handlers.truncate(mask);
    escaped
}
