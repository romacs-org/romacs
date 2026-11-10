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
    /// A job's claim (`romacs--isolate-claim'), above its handler's entry:
    /// signal search passes it by, and the job runs while it stands
    /// (`Vm::isolate_claim').
    Job(u64),
}
romacs_snapshot::snapshot_enum!(SignalHandler {
    SignalHandler::Condition(f0) => (f0),
    SignalHandler::Catch(f0) => (f0),
    SignalHandler::Bind { conditions, function, group } => (conditions, function, group),
    SignalHandler::Mask(f0) => (f0),
    SignalHandler::Boundary => (),
    SignalHandler::Job(f0) => (f0),
});

impl SignalHandler {
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        match self {
            Self::Condition(value) | Self::Catch(value) => visit(value),
            Self::Bind { conditions, function, .. } => { visit(conditions); visit(function); }
            Self::Mask(_) | Self::Boundary | Self::Job(_) => {}
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
/// The pending escape (`e.escape`) is engine state, traced, while the hook,
/// the handlers and the debugger run Lisp. Whether one replaced it.
pub(super) fn prepare_signal<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, hooked: bool, failed_depth: Option<usize>) -> bool {
    // Only a signal is the hook's, and only one not yet normalized searched:
    // a throw or a region's normal end routes with no Lisp at all.
    let signal = m.leaf((), |e, ()| matches!(e.escape, Some(Escape::Signal(..))));
    if !signal {
        return false;
    }
    let mut replaced = !hooked && signal_hook(m, gc.by_ref(), failed_depth);
    let signalled = m.leaf((), |e, ()| {
        let Some(&Escape::Signal(condition, data)) = e.escape.as_ref() else { return false };
        let object = e.vm.list_value(&[condition]);
        let index = object.index(3).unwrap();
        e.vm.conses.set_cdr(index, data);
        let conditions = e.vm.signal_conditions(condition);
        e.escape = Some(Escape::Error { condition, object, conditions, target: None });
        true
    });
    if signalled {
        replaced |= search(m, gc.by_ref());
        if !replaced && m.leaf((), |e, ()| debugger_wanted(e)) && call_debugger(m, gc.by_ref(), failed_depth) {
            prepare_signal(m, gc, false, None);
            replaced = true;
        }
    }
    replaced
}

/// Whether CONDITION's handler search ends at a compile-time expansion's
/// `Boundary': GNU signals it only at the form's reach, where
/// `signal-hook-function' hears of it, so it hears nothing now. A
/// `handler-bind' that matches first may leave before the boundary, so
/// the hook hears that signal now (and again at the reach, if it does not).
pub(super) fn speculative(e: &mut Engine<'_>, condition: Value) -> bool {
    let conditions = e.vm.signal_conditions(condition);
    let mut at = e.vm.signal_handlers.len();
    while at != 0 {
        at -= 1;
        match e.vm.signal_handlers[at] {
            SignalHandler::Mask(skip) => at = skip,
            SignalHandler::Boundary => return true,
            SignalHandler::Condition(choices) | SignalHandler::Bind { conditions: choices, .. }
                if matches(e.vm, conditions, choices) => return false,
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

/// What a handler search finds at one handler.
enum Found {
    /// The search ends here (a boundary, or a condition case that takes it).
    Stop,
    /// A mask: resume below it.
    Skip(usize),
    /// Nothing here.
    Next,
    /// A `handler-bind' handler to call: its function and the error object
    /// rooted in the frame, and its group.
    Bind(RootFrame, usize),
}
// SAFETY: a root frame and indices.
unsafe impl Plain for Found {}
fn search<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>) -> bool {
    let mut at = m.leaf((), |e, ()| e.vm.signal_handlers.len());
    while at != 0 {
        at -= 1;
        let found = m.leaf(at, |e, at| {
            let handler = e.vm.signal_handlers[at].clone();
            let Some(Escape::Error { condition, object, conditions, .. }) = e.escape.as_ref() else { return Found::Stop };
            let (condition, conditions, object) = (*condition, *conditions, *object);
            match handler {
                SignalHandler::Mask(skip) => Found::Skip(skip),
                SignalHandler::Boundary => {
                    if let Some(Escape::Error { target, .. }) = e.escape.as_mut() { *target = Some(at); }
                    Found::Stop
                }
                SignalHandler::Condition(choices) if matches(e.vm, conditions, choices) => {
                    if let Some(Escape::Error { target, .. }) = e.escape.as_mut() { *target = Some(at); }
                    Found::Stop
                }
                SignalHandler::Bind { conditions: choices, function, group } if matches(e.vm, conditions, choices) => {
                    e.vm.publish_signal_conditions(condition, conditions);
                    Found::Bind(e.vm.root_frame(&[function, object]), group)
                }
                _ => Found::Next,
            }
        });
        match found {
            Found::Stop => break,
            Found::Skip(skip) => at = skip,
            Found::Next => {}
            Found::Bind(frame, group) => {
                if handler_callback(m, gc.by_ref(), frame, group) { return true; }
            }
        }
    }
    false
}

/// A `handler-bind' handler's call: the engine suspended (its controls and
/// inline caches forwarded in place, its instruction allocations unmoved),
/// the reader boundary forwarding its registers; the handler's function and
/// the error object rooted in FRAME.
fn handler_callback<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, frame: RootFrame, group: usize) -> bool {
    let (mask, saved_depth, token) = m.leaf(group, |e, group| {
        let mask = e.vm.signal_handlers.len();
        e.vm.signal_handlers.push(SignalHandler::Mask(group));
        let saved_depth = std::mem::replace(&mut e.vm.eval_depth, e.depth_base + e.frames.len());
        (mask, saved_depth, e.vm.max_ensure_room(200))
    });
    let (escaped, hooked, depth) = lend::callback(m, gc.by_ref(), &frame, (), |e, (), result| {
        let replacement = e.vm.propagated_escape.take();
        let hooked = std::mem::take(&mut e.vm.propagated_escape_hooked);
        let depth = e.vm.propagated_eval_depth.take();
        e.fatal_image_failure |= e.vm.execution_fatal;
        e.code.resize_with(e.vm.functions.len(), Vec::new);
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
        (escaped, hooked, depth)
    });
    m.leaf(frame, |e, frame| e.vm.drop_frame(frame));
    if escaped { prepare_signal(m, gc, hooked, depth); }
    m.leaf((token, saved_depth, mask), |e, (token, saved_depth, mask)| {
        if let Some(old) = token { e.vm.restore_stack_limits(old); }
        e.vm.eval_depth = saved_depth;
        e.vm.signal_handlers.truncate(mask);
    });
    escaped
}
