//! The engine's lends: every way a running engine reaches a collection point.
//! Each one suspends the engine (its state and code moved to the Vm, its
//! registers set aside when Lisp runs nested), takes the permission
//! (`crate::brand::Gc`), and hands what the call returned straight to a leaf.
//! Only this module suspends an engine, so raw code cannot make a collection
//! look permitted to the funnel (`Vm::collect_marked`).
//!
//! What a nested call runs (`Nested`, the watched writes) is code not yet
//! converted: each callee roots what it is handed, as its file's census rows
//! say. The handoff is read from traced storage (a register, a `RootFrame`)
//! here, at the call.
use super::*;
use crate::brand::{Gc, Key, Mx, Plain};
use std::marker::PhantomData;

/// The crate's key (`crate::brand::Key`): named only here.
struct EngineKey;
// SAFETY: the one key; it reaches only this module's collecting operations.
unsafe impl Key for EngineKey {}

// SAFETY: a primitive is an index into the static primitive table.
unsafe impl Plain for Primitive {}

/// An engine in orchestration.
pub(super) type Orch<'e, 'b, 'a> = Mx<'e, 'b, Engine<'a>>;

/// An instruction of the running engine's code, valid for 'x: where
/// orchestration resumes, and what raw mode dispatches through
/// (`continue_at`).
#[derive(Clone, Copy)]
pub(super) struct Pc<'x>(*const Inst, PhantomData<&'x Inst>);
// SAFETY: a code address. Orchestration reads only the instruction's plain
// operands through it (`a`, `b`, `late_bind`, `late_write`); constants stay
// engine code, traced in place while the engine's state is with the Vm.
unsafe impl Plain for Pc<'_> {}
impl<'x> Pc<'x> {
    /// # Safety
    /// PC names an instruction of the running engine's code, and that code
    /// stays in place for 'x (lowered code is never moved while it runs).
    #[inline(always)]
    pub(super) unsafe fn new(pc: *const Inst) -> Self {
        Pc(pc, PhantomData)
    }
    #[inline(always)]
    pub(super) fn get(self) -> *const Inst {
        self.0
    }
    #[inline(always)]
    fn inst(self) -> &'x Inst {
        // SAFETY: `new`'s contract: a live instruction for 'x.
        unsafe { &*self.0 }
    }
    /// The instruction's first operand: its destination register, or a
    /// binding's slot.
    #[inline(always)]
    pub(super) fn a(self) -> usize {
        self.inst().a
    }
    /// The instruction's second operand: a binding's value register.
    #[inline(always)]
    pub(super) fn b(self) -> usize {
        self.inst().b
    }
    /// A late binding's slot and cell (Op::BindLate).
    pub(super) fn late_bind(self) -> (usize, usize) {
        match self.inst().op {
            Op::BindLate(slot, cell) => (slot, cell),
            _ => unreachable!("a late binding"),
        }
    }
    /// A late write's slot and value register (Op::WriteLate).
    pub(super) fn late_write(self) -> (usize, usize) {
        match self.inst().op {
            Op::WriteLate(_, reg, slot) => (slot, reg),
            _ => unreachable!("a late write"),
        }
    }
}

impl Engine<'_> {
    /// Hand this engine's state to the Vm across a nested call that may
    /// collect. Its registers stay where they are. The caller must reach the
    /// Vm through `self.vm` alone until `resume_code`.
    fn suspend_code(&mut self) {
        let record = Suspended {
            code: std::mem::take(&mut self.code),
            entries: std::mem::take(&mut self.entries),
            rest: std::mem::take(&mut self.rest),
            apply_function: self.apply_function.take(),
            controls: std::mem::take(&mut self.controls),
            escape: self.escape.take(),
            frames: std::mem::take(&mut self.frames),
            file: self.vm.suspended_stacks.len(),
            function: self.function,
            regs: self.regs,
            stack_used: std::mem::take(&mut self.stack_used),
            spread_end: std::mem::take(&mut self.spread_end),
        };
        self.vm.suspended.push(record);
    }
    /// Take this engine's state back, as the collector left it.
    fn resume_code(&mut self) {
        let record = self.vm.suspended.pop().expect("this engine's suspended state");
        self.code = record.code;
        self.entries = record.entries;
        self.rest = record.rest;
        self.apply_function = record.apply_function;
        self.controls = record.controls;
        self.escape = record.escape;
        self.frames = record.frames;
        // Down to its top, if a collection cleared above it meanwhile.
        self.stack_used = record.stack_used;
        self.spread_end = record.spread_end;
    }
    /// The current frame's offset in the register stack.
    #[inline(always)]
    fn frame_base(&self) -> usize {
        (self.regs as usize - self.vm.stack.as_ptr() as usize) / std::mem::size_of::<Value>()
    }
    /// Register I of the current frame, bounds-checked: for orchestration's
    /// leaves, off the hot path.
    #[inline(always)]
    pub(super) fn reg(&self, i: usize) -> Value {
        self.vm.stack[self.frame_base() + i]
    }
    /// Set register I of the current frame, bounds-checked.
    #[inline(always)]
    pub(super) fn set_reg(&mut self, i: usize, value: Value) {
        let at = self.frame_base() + i;
        self.vm.stack[at] = value;
    }
    /// RUN with only this engine's state and code suspended: the registers
    /// stay in place (a watcher, a reader body, a handler callback, a
    /// collection), so `e.regs` is still the frame's.
    fn code_suspended<T>(&mut self, run: impl FnOnce(&mut Vm) -> T) -> T {
        self.suspend_code();
        let out = run(self.vm);
        self.resume_code();
        out
    }
    /// RUN as a nested engine run: this engine suspended, its register file
    /// set aside (traced, its allocation unmoved). The run has a file of its
    /// own, so this one comes back as it was.
    fn suspended<T>(&mut self, run: impl FnOnce(&mut Vm) -> T) -> T {
        self.suspend_code();
        self.vm.suspend_stack();
        let out = run(self.vm);
        self.vm.resume_stack();
        self.resume_code();
        self.vm.end_nested_execution();
        self.code.resize_with(self.vm.functions.len(), Vec::new);
        out
    }
}

/// Lisp a suspended engine runs nested.
pub(super) enum Nested<'f> {
    /// Compile FORM and run it as an `eval` (the signal hook, the debugger).
    Form(&'f Form),
    /// Expand and compile the lazy function FUNCTION (`Vm::force_lazy`).
    Lazy(usize),
    /// `autoload-do-load' of FUNDEF and FUNCTION, rooted in the frame
    /// (FUNCTION first: the call is retried from it).
    Autoload(&'f RootFrame),
    /// The closure of a `(lambda ...)' list, rooted in the frame.
    LambdaList(&'f RootFrame),
    /// A collection's aftermath: its runner (`romacs--run-finalizers').
    Aftermath(Imm),
    /// The loader primitive P on the arguments rooted in the frame.
    Loader(Primitive, &'f RootFrame),
}
impl Nested<'_> {
    fn run(self, vm: &mut Vm) -> Result<Value> {
        stacker::maybe_grow(512 * 1024, 2 * 1024 * 1024, || match self {
            Nested::Form(form) => {
                let function = vm.compile(form)?;
                vm.propagate_next = true;
                let result = vm.execute_raw(function);
                vm.propagate_next = false;
                result
            }
            Nested::Lazy(function) => vm.force_lazy(function).map(|()| Value::NIL),
            Nested::Autoload(frame) => {
                let (function, fundef) = (vm.rooted(frame, 0), vm.rooted(frame, 1));
                vm.autoload_do_load(fundef, function, Value::NIL)
            }
            Nested::LambdaList(frame) => {
                let list = vm.rooted(frame, 0);
                vm.lambda_list_closure(list)
            }
            Nested::Aftermath(runner) => vm.run_aftermath(runner.get(), true),
            Nested::Loader(p, frame) => {
                let args = vm.rooted_values(frame).to_vec();
                vm.loader_primitive(p, &args)
            }
        })
    }
}

/// Run CALL nested, then THEN on its result.
pub(super) fn nested<'a, 'b, B: Plain, R: Plain>(m: &mut Orch<'_, 'b, 'a>, _gc: Gc<'_, 'b>, call: Nested<'_>, b: B,
    then: impl FnOnce(&mut Engine<'a>, B, Result<Value>) -> R + Copy + 'static) -> R {
    let e = m.raw(&EngineKey);
    let result = e.suspended(|vm| call.run(vm));
    then(e, b, result)
}

/// Lisp that cannot collect, run nested from raw mode: the hold makes the
/// funnel refuse whatever it reaches, so the caller may keep Values in Rust
/// locals across it (a lazy body's expansion, under its builder's hold too).
/// Named and counted: a deferred collection is latency debt.
pub(super) fn deferred(e: &mut Engine<'_>, call: Nested<'_>) -> Result<Value> {
    e.vm.gc_holds += 1;
    let out = e.suspended(|vm| call.run(vm));
    e.vm.gc_holds -= 1;
    out
}

/// Collect at this engine's poll: whether it collected (false when refused,
/// a hold open or another engine running unsuspended).
pub(super) fn collect<'b>(m: &mut Orch<'_, 'b, '_>, _gc: Gc<'_, 'b>, until_key: bool) -> bool {
    m.raw(&EngineKey).code_suspended(|vm| vm.collect_until_key(&mut [], until_key))
}

/// A variable write whose watchers may run Lisp, and so collect.
pub(super) enum Write<'f> {
    /// SLOT's variable set to register REG of the current frame.
    Set(usize, usize),
    /// SLOT's variable bound to the value rooted in the frame at the index.
    Bind(usize, &'f RootFrame, usize),
}
/// Make WRITE with this engine suspended, the writer's own Values rooted by
/// it (`rooted_write`), then THEN on its outcome.
pub(super) fn watched<'a, 'b, B: Plain, R: Plain>(m: &mut Orch<'_, 'b, 'a>, _gc: Gc<'_, 'b>, write: Write<'_>, b: B,
    then: impl FnOnce(&mut Engine<'a>, B, Result<()>) -> R + Copy + 'static) -> R {
    let e = m.raw(&EngineKey);
    // Handed to the write at once; suspending the code leaves it in place.
    let set = match write {
        Write::Set(_, reg) => e.reg(reg),
        Write::Bind(..) => Value::NIL,
    };
    let outcome = e.code_suspended(|vm| {
        let rooted = std::mem::replace(&mut vm.rooted_write, true);
        let outcome = match write {
            Write::Set(slot, _) => vm.set_variable_value(slot, set),
            Write::Bind(slot, frame, i) => {
                let value = vm.rooted(frame, i);
                vm.bind_variable(slot, value)
            }
        };
        vm.rooted_write = rooted;
        outcome
    });
    then(e, b, outcome)
}
/// Unbind N bindings, their unlet watchers free to collect; the first
/// restore that signals is consumed and its escape handed to THEN, like
/// unbind_to's pop.
pub(super) fn unbind<'a, 'b, B: Plain, R: Plain>(m: &mut Orch<'_, 'b, 'a>, _gc: Gc<'_, 'b>, n: usize, b: B,
    then: impl FnOnce(&mut Engine<'a>, B, Option<Escape>) -> R + Copy + 'static) -> R {
    let e = m.raw(&EngineKey);
    let escape = e.code_suspended(|vm| {
        let rooted = std::mem::replace(&mut vm.rooted_write, true);
        let mut escape = None;
        for _ in 0..n {
            if vm.unbind_one().is_err() {
                escape = vm.propagated_escape.take();
                break;
            }
        }
        vm.rooted_write = rooted;
        escape
    });
    then(e, b, escape)
}

/// Pop N bindings as an engine's exit does (`Vm::unwind`): a restore that
/// signals keeps its value and the escape in flight stays; the unlet watchers
/// are free to collect.
pub(super) fn unwind<'b>(m: &mut Orch<'_, 'b, '_>, _gc: Gc<'_, 'b>, n: usize) {
    m.raw(&EngineKey).code_suspended(|vm| {
        let rooted = std::mem::replace(&mut vm.rooted_write, true);
        vm.unwind(n);
        vm.rooted_write = rooted;
    });
}

/// A generated primitive whose body reads (`read`, `read-from-string`), and
/// so runs Lisp: TARGET on the ARGC arguments at register offset START, its
/// step handed to THEN.
pub(super) fn read<'a, 'b, B: Plain, R: Plain>(m: &mut Orch<'_, 'b, 'a>, _gc: Gc<'_, 'b>, (target, start, argc): (Imm, usize, usize), b: B,
    then: impl FnOnce(&mut Engine<'a>, B, std::result::Result<prim::Step, Option<(Value, Value)>>) -> R + Copy + 'static) -> R {
    let e = m.raw(&EngineKey);
    let step = e.code_suspended(|vm| vm.call_generated(target.get(), start, argc));
    then(e, b, step)
}

/// A `handler-bind' handler: FUNCTION on OBJECT (both rooted in the frame)
/// through the reader trampoline, which sets the registers aside itself.
pub(super) fn callback<'a, 'b, B: Plain, R: Plain>(m: &mut Orch<'_, 'b, 'a>, _gc: Gc<'_, 'b>, frame: &RootFrame, b: B,
    then: impl FnOnce(&mut Engine<'a>, B, Result<Value>) -> R + Copy + 'static) -> R {
    let e = m.raw(&EngineKey);
    let result = e.code_suspended(|vm| {
        let (function, object) = (vm.rooted(frame, 0), vm.rooted(frame, 1));
        vm.reader_call(function, &[object])
    });
    then(e, b, result)
}
