//! Pre-unwind signal search shared by ordinary and reentrant engines.
use super::*;

#[derive(Clone)]
pub(crate) enum SignalHandler {
    Condition(Value),
    Catch(Value),
    Bind { conditions: Value, function: Value, group: usize },
    Mask(usize),
}
romacs_snapshot::snapshot_enum!(SignalHandler {
    SignalHandler::Condition(f0) => (f0),
    SignalHandler::Catch(f0) => (f0),
    SignalHandler::Bind { conditions, function, group } => (conditions, function, group),
    SignalHandler::Mask(f0) => (f0),
});

impl SignalHandler {
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        match self {
            Self::Condition(value) | Self::Catch(value) => visit(value),
            Self::Bind { conditions, function, .. } => { visit(conditions); visit(function); }
            Self::Mask(_) => {}
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
    if !hooked && let Some(Escape::Signal(condition, data)) = e.escape.as_ref()
        && let Some(replacement) = signal_hook(e, *condition, *data, failed_depth)
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
    }
    replaced
}

fn search(e: &mut Engine<'_>) -> bool {
    let mut at = e.vm.signal_handlers.len();
    while at != 0 {
        at -= 1;
        let handler = e.vm.signal_handlers[at].clone();
        let Some(Escape::Error { condition, object, conditions, .. }) = e.escape.as_ref() else { break; };
        let (condition, conditions, object) = (*condition, *conditions, *object);
        match handler {
            SignalHandler::Mask(skip) => at = skip,
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
