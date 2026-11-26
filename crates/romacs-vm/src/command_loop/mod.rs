//! GNU keyboard.c's key reader in Rust: `read_key_sequence` (`keyseq.rs`),
//! `read_char` (`read_char.rs`) with the keyremap scans (`keyremap.rs`),
//! over the input stream (`stream.rs`). The command loop's step, its hooks,
//! input methods, filters and translation functions stay Lisp: the reader
//! calls them.
//!
//! The reader is orchestration in the `crate::brand` style, as the host
//! wait is (`host_input.rs`): it holds the permission and never a `Value`
//! across a Lisp call. Its state lives in traced storage, the stream's
//! `KeyseqFrame`s and `Reads`, or a `RootFrame` its caller releases; leaves
//! read it back after each call. An escape out of a call comes back as
//! `Err(())` with the escape waiting in `propagated_escape`, and the exit
//! helper (`keyseq::read`) runs the read's cleanup before handing it on.
use crate::brand::{Gc, Mx};
use crate::gc::RootFrame;
use crate::prim::{Fault, ReadCall, land};
use crate::{Value, Vm, threaded};

pub(crate) mod stream;
mod keyremap;
mod keyseq;
mod read_char;
#[cfg(test)]
mod reader_tests;

pub(crate) use keyseq::{read_key_sequence, read_key_sequence_vs};
pub(crate) use read_char::read_filtered_event;
pub(crate) use stream::{MAIN, Reads, Store, Streams};

impl Vm {
    /// The input streams.
    pub(crate) fn streams(&self) -> &Streams {
        &self.streams
    }
    /// The input streams, to write through `Streams::store`.
    pub(crate) fn streams_mut(&mut self) -> &mut Streams {
        &mut self.streams
    }
}

/// A step of the reader: `Err(())` when an escape waits in
/// `propagated_escape` for the reader's caller.
type Done<T> = std::result::Result<T, ()>;
/// The reader's orchestration over the VM.
type M<'m, 'b> = Mx<'m, 'b, Vm>;

/// ERROR, an engine's failure text, as the escape a Lisp caller sees; one
/// already pending stays.
fn escape(vm: &mut Vm, error: String) {
    if vm.mutator.propagated_escape.is_none() {
        signal(vm, Fault::Error(error));
    }
}
/// FAULT as the escape a Lisp caller sees.
fn signal(vm: &mut Vm, fault: Fault) {
    if !matches!(fault, Fault::Escape) {
        let (condition, data) = vm.fault(fault);
        vm.mutator.propagated_escape = Some(crate::threaded::Escape::Signal(condition, data));
    }
}
/// The current value of the variable NAME, nil when void.
fn var(vm: &mut Vm, name: &str) -> Value {
    let slot = vm.slot(name);
    match vm.variable_value(slot) {
        Ok(value) if value != Value::UNBOUND => value,
        _ => Value::NIL,
    }
}
/// Set the variable NAME as GNU's C assigns it; a watched variable is set
/// by a request after the leaf (`flush`), so its watchers run as they did
/// when the reader was Lisp.
fn store(vm: &mut Vm, name: &str, value: Value) -> Done<()> {
    let slot = vm.slot(name);
    if vm.variable_trapped(vm.indirect_variable(slot)) {
        vm.reads.writes.push((slot, value));
        return Ok(());
    }
    if let Err(error) = vm.assign_variable(slot, value) {
        escape(vm, error);
        return Err(());
    }
    Ok(())
}
/// A leaf of the reader, then the writes it left for watchers (`store`).
fn step<'b, A: crate::brand::Plain, R: crate::brand::Plain>(m: &mut M<'_, 'b>, gc: Gc<'_, 'b>, a: A,
    f: impl FnOnce(&mut Vm, A) -> Done<R> + Copy + 'static) -> Done<R> {
    let out = m.leaf(a, f);
    if out.is_err() {
        m.leaf((), |vm, ()| vm.reads.writes.clear());
        return out;
    }
    flush(m, gc)?;
    out
}
/// The writes `store` left, each a request whose watchers may run Lisp.
fn flush<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>) -> Done<()> {
    loop {
        let next = m.leaf((), |vm, ()| {
            if vm.reads.writes.is_empty() { return None; }
            let (slot, value) = vm.reads.writes.remove(0);
            Some((slot, vm.root_frame(&[value])))
        });
        let Some((slot, frame)) = next else { return Ok(()) };
        let written = threaded::write_from(m, gc.by_ref(), slot, &frame, 0);
        m.leaf((frame, written), |vm, (frame, written)| {
            vm.drop_frame(frame);
            vm.reads.callouts += 1;
            written.map_err(|error| escape(vm, error))
        })?;
    }
}
/// Call the function and arguments ARGS gives: its value in `Reads::result`.
fn call<'b, A: crate::brand::Plain>(m: &mut M<'_, 'b>, gc: Gc<'_, 'b>, a: A,
    args: impl FnOnce(&mut Vm, A) -> Done<Vec<Value>> + Copy + 'static) -> Done<()> {
    let frame = m.leaf(a, move |vm, a| args(vm, a).map(|values| vm.root_frame(&values)))?;
    called(m, gc, frame)
}
/// The function rooted first in FRAME called on the rest, FRAME released:
/// its value in `Reads::result`.
fn called<'b>(m: &mut M<'_, 'b>, gc: Gc<'_, 'b>, frame: RootFrame) -> Done<()> {
    let outcome = threaded::call_frame(m, gc, &frame, (), |vm, (), value| match value {
        Ok(value) => {
            vm.reads.result = value;
            Ok(())
        }
        Err(error) => {
            escape(vm, error);
            Err(())
        }
    });
    m.leaf(frame, |vm, frame| {
        vm.drop_frame(frame);
        vm.reads.callouts += 1;
    });
    outcome
}
/// `call` with the variables NAMES bound to the values BIND gives, as `let`
/// binds them, around the call.
fn call_bound<'b, A: crate::brand::Plain + Copy>(m: &mut M<'_, 'b>, gc: Gc<'_, 'b>, names: &'static [&'static str], a: A,
    bind: impl FnOnce(&mut Vm, A) -> Done<Vec<Value>> + Copy + 'static,
    args: impl FnOnce(&mut Vm, A) -> Done<Vec<Value>> + Copy + 'static) -> Done<()> {
    let values = m.leaf(a, move |vm, a| bind(vm, a).map(|values| vm.root_frame(&values)))?;
    bound(m, gc, names, values, |m, gc| call(m, gc, a, args))
}
/// BODY with the variables NAMES bound to VALUES, as `let` binds them:
/// unbound after it, or unwound when it escapes. VALUES is released.
fn bound<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, names: &[&'static str], values: RootFrame,
    body: impl FnOnce(&mut M<'_, 'b>, Gc<'_, 'b>) -> Done<()>) -> Done<()> {
    let mark = m.leaf((), |vm, ()| vm.mutator.dynamic.len());
    let mut bound = Ok(());
    for (i, name) in names.iter().enumerate() {
        let slot = m.leaf(*name, Vm::slot);
        bound = threaded::bind_from(m, gc.by_ref(), slot, &values, i);
        if bound.is_err() { break; }
    }
    let outcome = match bound {
        Ok(()) => body(m, gc.by_ref()),
        Err(error) => {
            m.leaf(error, escape);
            Err(())
        }
    };
    let unbound = if outcome.is_ok() {
        threaded::unbind_to(m, gc.by_ref(), mark)
    } else {
        threaded::unwind_to(m, gc, mark);
        Ok(())
    };
    m.leaf((values, outcome, unbound), |vm, (values, outcome, unbound)| {
        vm.drop_frame(values);
        outcome?;
        unbound.map_err(|error| escape(vm, error))
    })
}
/// The Lisp half of the command loop (`prims/keyboard.rs`), evaluated now
/// unless it is: the reader calls its host and batch waits, its echo and
/// its macro recording.
fn ensure_lisp_half<'b>(m: &mut M<'_, 'b>, gc: Gc<'_, 'b>) -> Done<()> {
    let installed = m.leaf((), |vm, ()| {
        let mut cx = crate::prim::Leaf::new(vm);
        let probe = cx.intern("romacs--read-host-input");
        cx.symbol_function(probe).is_ok_and(|function| function != Value::NIL)
    });
    if installed { return Ok(()); }
    call(m, gc, (), |vm, ()| {
        let form = crate::prim::prims::keyboard::lisp_half_form(vm).map_err(|fault| signal(vm, fault))?;
        let eval = vm.intern("eval");
        Ok(vec![eval, form, Value::TRUE])
    })
}
/// A string's character as `aref` gives it: a raw byte (`CHAR_BYTE8_P`) is
/// the byte itself.
fn byte_char(c: i64) -> i64 {
    if (0x3f_ff80..=0x3f_ffff).contains(&c) { c - 0x3f_ff00 } else { c }
}
/// A reading primitive's end: its value, or the escape that waits, to OUT.
fn landed(vm: &mut Vm, out: &RootFrame, outcome: Done<Value>) -> bool {
    land(vm, out, outcome.map_err(|()| Fault::Escape))
}
/// The argument at I of a reading primitive's call, nil past its count.
fn arg(vm: &Vm, (start, argc): (usize, usize), i: usize) -> Value {
    if i < argc { vm.mutator.stack[start + i] } else { Value::NIL }
}
/// A reading primitive's arguments' place, plain.
fn place(call: &ReadCall<'_, '_, '_, '_>) -> (usize, usize) {
    (call.start, call.argc)
}
