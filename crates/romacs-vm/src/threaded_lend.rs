//! The engine's lends: every way a running engine reaches a collection point.
//! Each one suspends the engine (its state and code moved to the Vm, its
//! registers set aside when Lisp runs nested), takes the permission
//! (`crate::brand::Gc`), and hands what the call returned straight to a leaf.
//! Only this module suspends an engine, so raw code cannot make a collection
//! look permitted to the funnel (`Vm::collect_marked`).
//!
//! What a nested call runs (`Nested`, the watched writes) is mostly code not yet
//! converted: each callee roots what it is handed, as its file's census rows
//! say. The handoff is read from traced storage (a register, a `RootFrame`)
//! here, at the call. A load is converted: it goes on under this permission.
//!
//! The requests over a VM below are the same thing without an engine: what
//! orchestration over a `Vm` (`crate::loader`, `crate::command`) may ask.
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

/// An embedder's request, from no engine: BODY holds the top-level permission
/// (`Gc::top`), and what it returns is plain. Counted in `rooting_brand_rules`.
pub(crate) fn toplevel<A: Plain, N: Plain + 'static>(vm: &mut Vm, a: A, body: for<'m, 'b, 'g> fn(Mx<'m, 'b, Vm>, Gc<'g, 'b>, A) -> N) -> N {
    crate::brand::orchestrate(crate::brand::Eng::lend(vm, Gc::top(&EngineKey)), a, body, |_, next| next)
}
/// Orchestration over the VM under a permission already held: a suspended
/// engine's nested load.
fn over_vm<'g, A: Plain, N: Plain + 'static>(vm: &'g mut Vm, gc: Gc<'g, '_>, a: A, body: for<'m, 'b, 'p> fn(Mx<'m, 'b, Vm>, Gc<'p, 'b>, A) -> N) -> N {
    crate::brand::orchestrate(crate::brand::Eng::lend(vm, gc), a, body, |_, next| next)
}
/// Evaluate FORM as a request, THEN consuming its value at once.
pub(crate) fn eval_form<'b, B: Plain, R: Plain>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, form: &Form, b: B,
    then: impl FnOnce(&mut Vm, B, Result<Value>) -> R + Copy + 'static) -> R {
    m.request(&EngineKey, gc, |vm| vm.eval_form_raw(form), b, then)
}
/// Evaluate the text SOURCE as a request, THEN consuming its value at once.
pub(crate) fn eval_text<'b, B: Plain, R: Plain>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, source: &str, b: B,
    then: impl FnOnce(&mut Vm, B, Result<Value>) -> R + Copy + 'static) -> R {
    m.request(&EngineKey, gc, |vm| vm.eval_source_raw(source), b, then)
}
/// Run the code of FUNCTION as a request, THEN consuming its value at once.
pub(crate) fn run_function<'b, B: Plain, R: Plain>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, function: usize, b: B,
    then: impl FnOnce(&mut Vm, B, Result<Value>) -> R + Copy + 'static) -> R {
    m.request(&EngineKey, gc, |vm| vm.execute_raw(function), b, then)
}
/// Compile FORM as a request: the function that runs it.
pub(crate) fn compile_form<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, form: &Form) -> Result<usize> {
    m.request(&EngineKey, gc, |vm| vm.compile_request(form).map(|(function, _)| function), (), |_, (), compiled| compiled)
}
/// FORM's macro expansion, a macro's autoload loaded first; None when it has none.
pub(crate) fn expand_form<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, form: &Form) -> Result<Option<Form>> {
    m.request(&EngineKey, gc, |vm| vm.expand_macro(form), (), |_, (), expanded| expanded)
}
/// The next form of SOURCE, its value, offset and end; reading can call Lisp
/// (a `\N{NAME}` character). THEN consumes the value at once.
pub(crate) fn read_next<'b, B: Plain, R: Plain>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, source: &[u8], b: B,
    then: impl FnOnce(&mut Vm, B, Result<Option<(Value, usize, usize)>>) -> R + Copy + 'static) -> R {
    m.request(&EngineKey, gc, |vm| vm.read_internal_span(source), b, then)
}
/// The function rooted first in FRAME, called on the rest, THEN consuming its
/// value at once.
pub(crate) fn call_frame<'b, B: Plain, R: Plain>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, frame: &RootFrame, b: B,
    then: impl FnOnce(&mut Vm, B, Result<Value>) -> R + Copy + 'static) -> R {
    m.request(&EngineKey, gc, |vm| {
        let values = vm.rooted_values(frame).to_vec();
        vm.reader_call(values[0], &values[1..])
    }, b, then)
}
/// A writer whose Values are rooted: its watchers are free to collect.
fn rooted_writer<T>(vm: &mut Vm, run: impl FnOnce(&mut Vm) -> T) -> T {
    let rooted = std::mem::replace(&mut vm.rooted_write, true);
    let out = run(vm);
    vm.rooted_write = rooted;
    out
}
/// Bind SLOT's variable to the value rooted in FRAME at I, as `let` does.
pub(crate) fn bind_from<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, slot: usize, frame: &RootFrame, i: usize) -> Result<()> {
    m.request(&EngineKey, gc, |vm| rooted_writer(vm, |vm| {
        let value = vm.rooted(frame, i);
        vm.bind_variable(slot, value)
    }), (), |_, (), bound| bound)
}
/// Set SLOT's variable to the value rooted in FRAME at I, as `set` does.
pub(crate) fn write_from<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, slot: usize, frame: &RootFrame, i: usize) -> Result<()> {
    m.request(&EngineKey, gc, |vm| rooted_writer(vm, |vm| {
        let value = vm.rooted(frame, i);
        vm.set_variable_value(slot, value)
    }), (), |_, (), written| written)
}
/// GNU `unbind_to` on normal completion, down to MARK (`Vm::unbind`).
pub(crate) fn unbind_to<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, mark: usize) -> Result<()> {
    m.request(&EngineKey, gc, |vm| rooted_writer(vm, |vm| vm.unbind(vm.dynamic.len() - mark)), (), |_, (), unbound| unbound)
}
/// Unwinding under an escape, down to MARK (`Vm::unwind`).
pub(crate) fn unwind_to<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, mark: usize) {
    m.request(&EngineKey, gc, |vm| rooted_writer(vm, |vm| vm.unwind(vm.dynamic.len() - mark)), (), |_, (), ()| ());
}

/// Lisp a suspended engine runs nested.
pub(super) enum Nested<'f> {
    /// Compile FORM and run it as an `eval` (the signal hook, the debugger).
    Form(&'f Form),
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
    fn run(self, vm: &mut Vm, gc: Gc<'_, '_>) -> Result<Value> {
        stacker::maybe_grow(512 * 1024, 2 * 1024 * 1024, || match self {
            Nested::Form(form) => {
                let function = vm.compile(form)?;
                vm.propagate_next = true;
                let result = vm.execute_raw(function);
                vm.propagate_next = false;
                result
            }
            Nested::Autoload(frame) => {
                let (function, fundef) = (vm.rooted(frame, 0), vm.rooted(frame, 1));
                let args = vm.autoload_args(fundef, function, Value::NIL);
                let outcome = over_vm(vm, gc, &args, |mut m, gc, args| crate::autoload::do_load(&mut m, gc, args));
                vm.autoload_result(args, outcome)
            }
            Nested::LambdaList(frame) => {
                let list = vm.rooted(frame, 0);
                vm.lambda_list_closure(list)
            }
            Nested::Aftermath(runner) => vm.run_aftermath(runner.get(), true),
            Nested::Loader(p, frame) if crate::loader::loads(p) => {
                let out = vm.root_frame(&[Value::NIL]);
                let outcome = over_vm(vm, gc, (p, frame, &out), |mut m, gc, (p, args, out)| crate::loader::load_primitive(&mut m, gc, p, args, (out, 0)));
                let [value] = vm.release_frame(out);
                outcome.map(|()| value)
            }
            Nested::Loader(p, frame) => {
                let args = vm.rooted_values(frame).to_vec();
                vm.loader_primitive(p, &args)
            }
        })
    }
}

/// Run CALL nested, then THEN on its result.
pub(super) fn nested<'a, 'b, B: Plain, R: Plain>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, call: Nested<'_>, b: B,
    then: impl FnOnce(&mut Engine<'a>, B, Result<Value>) -> R + Copy + 'static) -> R {
    let e = m.raw(&EngineKey);
    let result = e.suspended(|vm| call.run(vm, gc));
    then(e, b, result)
}

/// Expand and compile the lazy function FUNCTION (`Vm::force_lazy`), nested from
/// raw mode: the hold makes the funnel refuse whatever it reaches, so the caller
/// may keep Values in Rust locals across it (under its builder's hold too).
/// Named and counted: a deferred collection is latency debt.
pub(super) fn deferred(e: &mut Engine<'_>, function: usize) -> Result<()> {
    e.vm.gc_holds += 1;
    let out = e.suspended(|vm| stacker::maybe_grow(512 * 1024, 2 * 1024 * 1024, || vm.force_lazy(function)));
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
    let outcome = e.code_suspended(|vm| rooted_writer(vm, |vm| match write {
        Write::Set(slot, _) => vm.set_variable_value(slot, set),
        Write::Bind(slot, frame, i) => {
            let value = vm.rooted(frame, i);
            vm.bind_variable(slot, value)
        }
    }));
    then(e, b, outcome)
}
/// Write I of a queue's FRAME (`Vm::take_deferred`), this engine suspended, its
/// watchers free to collect; then THEN on its outcome.
pub(super) fn deferred_write<'a, 'b, B: Plain, R: Plain>(m: &mut Orch<'_, 'b, 'a>, _gc: Gc<'_, 'b>,
    (frame, i): (&RootFrame, usize), b: B,
    then: impl FnOnce(&mut Engine<'a>, B, std::result::Result<(), prim::Fault>) -> R + Copy + 'static) -> R {
    let e = m.raw(&EngineKey);
    let outcome = e.code_suspended(|vm| rooted_writer(vm, |vm| vm.perform_deferred(frame, i)));
    then(e, b, outcome)
}
/// Unbind N bindings, their unlet watchers free to collect; the first
/// restore that signals is consumed and its escape handed to THEN, like
/// unbind_to's pop.
pub(super) fn unbind<'a, 'b, B: Plain, R: Plain>(m: &mut Orch<'_, 'b, 'a>, _gc: Gc<'_, 'b>, n: usize, b: B,
    then: impl FnOnce(&mut Engine<'a>, B, Option<Escape>) -> R + Copy + 'static) -> R {
    let e = m.raw(&EngineKey);
    let escape = e.code_suspended(|vm| rooted_writer(vm, |vm| {
        for _ in 0..n {
            if vm.unbind_one().is_err() {
                return vm.propagated_escape.take();
            }
        }
        None
    }));
    then(e, b, escape)
}

/// Pop N bindings as an engine's exit does (`Vm::unwind`): a restore that
/// signals keeps its value and the escape in flight stays; the unlet watchers
/// are free to collect.
pub(super) fn unwind<'b>(m: &mut Orch<'_, 'b, '_>, _gc: Gc<'_, 'b>, n: usize) {
    m.raw(&EngineKey).code_suspended(|vm| rooted_writer(vm, |vm| vm.unwind(n)));
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
