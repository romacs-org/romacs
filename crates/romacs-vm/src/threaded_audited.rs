//! The audited core of the threaded engine (lisp-speed.md §13.6): every
//! `unsafe` the typed handlers (`threaded_hot.rs`, which forbids it) rely
//! on, behind capabilities they cannot forge, keep or mix.
//!
//! The trusted base is this module, `threaded_code.rs` (validation),
//! `registers.rs` (the files' one owner) and `threaded.rs` (lowering, and
//! the legacy handlers, `unsafe fn`s with their own contracts). The typed
//! handlers' bodies are not trusted: whatever safe code they hold, they
//! reach the engine only through what `Hot`, `Regs`, `Pc` and `Next` offer
//! here. Each site names the invariant it rests on:
//!
//! - I1, the file: `WINDOW` (256) initialised slots lie above every window
//!   (`frame_room`), so a 257-slot view from the base of a frame one register
//!   wide or more stays in its file. The file moves only by growth, and
//!   `Registers` alone changes its allocation (I6), neither reachable
//!   through `Hot`.
//! - I2 and I5, windows: a view borrows its `Regs` and the `Hot` exclusively,
//!   so nothing else reads or writes the file while it lives.
//! - I3, validated code: a `Code` is made only by `threaded_code.rs`, which
//!   holds every register operand inside its frame and every branch inside
//!   the code, ends it with `terminator`, whose handler is here, and installs
//!   every typed handler itself, from the operation it checked, so the
//!   operands a typed handler reads are the ones validated.
//! - I4, instruction pointers: a `Pc` points into a `Code` that outlives the
//!   handler holding it: code is replaced or cleared only while no engine
//!   runs it (`Program::retire_code`, `lower_function`), and `Hot` reaches
//!   no code.
//! - I7, one run: `Hot`, `Regs`, `Pc`, `Wide` and `Next` carry one
//!   invariant brand `'f`, which `enter` picks for each handler call; a body
//!   is generic over it, so it can neither keep a capability past its call
//!   (none is `'static`) nor pair two calls' capabilities.
use super::{fail, next, poll_resume, Eng, Handler, Inst, Value};
use std::marker::PhantomData;

/// One handler call's brand (I7): invariant in `'f`.
type Brand<'f> = PhantomData<fn(&'f ()) -> &'f ()>;

/// The engine as a typed handler sees it: its budget and the conses it
/// reads, and nothing that frees or moves a register file or code (I1, I4).
pub(super) struct Hot<'f, 'x, 'a> {
    e: Eng<'x, 'a>,
    _f: Brand<'f>,
}

/// The running frame's registers: one per engine, moved along dispatch, made
/// only here, and not `Copy`, so none outlives the handler it was given to.
#[repr(transparent)]
pub(super) struct Regs<'f>(*mut Value, Brand<'f>);

/// The running instruction. Operands are read by value, so no reference into
/// the code outlives a read, and a collection may rewrite the code's Values.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub(super) struct Pc<'f>(*const Inst, Brand<'f>);

/// A register operand of a frame wider than a byte, which validation
/// proved lies inside the frame: made only from a `Pc`'s operand.
#[derive(Clone, Copy)]
pub(super) struct Wide<'f>(usize, Brand<'f>);

/// Where an instruction goes. It can only be dispatched to, so a handler
/// takes one step at most and reads nothing through it.
#[repr(transparent)]
pub(super) struct Next<'f>(*const Inst, Brand<'f>);

impl Hot<'_, '_, '_> {
    /// Spend one unit of the engine's budget: whether it is gone, a poll due.
    #[inline]
    pub(super) fn spend(&mut self) -> bool {
        self.e.budget -= 1;
        self.e.budget < 0
    }
    /// Cons I's car and cdr, if the heap holds a cons at I.
    #[inline]
    pub(super) fn pair(&self, i: usize) -> Option<(Value, Value)> {
        self.e.vm.conses.get(i).copied()
    }
}

impl<'f> Regs<'f> {
    /// The frame's base, the capability spent: what dispatch hands on.
    #[inline]
    fn into_raw(self) -> *mut Value {
        self.0
    }
    /// 257 slots from the frame's base: a byte operand and the register after
    /// it, a binary kernel's second argument. The view borrows the engine
    /// exclusively as well as this capability, so nothing that goes through
    /// the engine, a shared read of the file included, runs while it lives.
    #[inline]
    pub(super) fn window<'w>(&'w mut self, _hot: &'w mut Hot<'f, '_, '_>) -> &'w mut [Value; 257] {
        // UNSAFE(a frame is an offset into its engine's register file, which no reference type can name)
        // SAFETY: I3: a typed handler's operands are validated below its
        // frame's width, so the frame is one register wide or more, and I1
        // puts 256 initialised slots past it; I5 makes this the one reference
        // to them while it lives, and I7 makes this run's file its frame's.
        unsafe { &mut *self.0.cast::<[Value; 257]>() }
    }
    /// Register AT of a frame wider than a byte: what its `Wide` operand
    /// names, read or written through the slot. Borrows as `window` does.
    #[inline]
    pub(super) fn slot<'w>(&'w mut self, _hot: &'w mut Hot<'f, '_, '_>, at: Wide<'f>) -> &'w mut Value {
        // UNSAFE(a frame is an offset into its engine's register file, which no reference type can name)
        // SAFETY: I3: `code::validate` proved AT below the frame's width, and
        // I1 puts that width and 256 initialised slots past the base; I5
        // makes this the one reference to it while it lives, and I7 makes AT
        // an operand of this frame's code.
        unsafe { &mut *self.0.add(at.0) }
    }
}

impl Next<'_> {
    /// The instruction, the capability spent: what dispatch hands on.
    #[inline]
    fn into_raw(self) -> *const Inst {
        self.0
    }
}

impl<'f> Pc<'f> {
    #[inline]
    fn inst(&self) -> &Inst {
        // UNSAFE(a Pc is dispatch's raw instruction pointer: a reference would tie each handler to its code's lifetime)
        // SAFETY: I3, I4 and I7: a Pc is made from an instruction of validated
        // code that outlives its handler's call, and this reference never
        // leaves this module.
        unsafe { &*self.0 }
    }
    /// The byte operands, for typed handlers (`code::bytes`).
    #[inline]
    pub(super) fn a(self) -> usize {
        self.inst().a8 as usize
    }
    #[inline]
    pub(super) fn b(self) -> usize {
        self.inst().b8 as usize
    }
    #[inline]
    pub(super) fn d(self) -> usize {
        self.inst().d8 as usize
    }
    /// The full operands, for frames wider than a byte (`Regs::slot`).
    #[inline]
    pub(super) fn wide_a(self) -> Wide<'f> {
        Wide(self.inst().a, PhantomData)
    }
    #[inline]
    pub(super) fn wide_b(self) -> Wide<'f> {
        Wide(self.inst().b, PhantomData)
    }
    #[inline]
    pub(super) fn value(self) -> Value {
        self.inst().value
    }
    #[inline]
    fn step(self, distance: isize) -> Next<'f> {
        // UNSAFE(a branch is a pointer offset within validated code, as dispatch takes it)
        // SAFETY: I3: every instruction's branch distance lands in its code,
        // and 1 does too, as validated code ends in the terminator and no
        // typed handler is handed the terminator's Pc.
        Next(unsafe { self.0.offset(distance) }, PhantomData)
    }
    /// The following instruction.
    #[inline]
    pub(super) fn next(self) -> Next<'f> {
        self.step(1)
    }
    /// This instruction's branch target (`Inst::branch`).
    #[inline]
    pub(super) fn branch(self) -> Next<'f> {
        self.step(self.inst().branch as isize)
    }
    /// Whether TO runs backward from here: a loop's poll.
    #[inline]
    pub(super) fn is_back(self, to: &Next<'f>) -> bool {
        to.0 <= self.0
    }
}

/// HANDLER on the frame R and the instruction AT: every way out of a typed
/// handler is this one call.
#[inline]
fn call(handler: Handler, hot: Hot<'_, '_, '_>, r: Regs<'_>, at: *const Inst) -> Value {
    let r = r.into_raw();
    // UNSAFE(a Handler is an unsafe fn pointer: the threaded engine's calling convention)
    // SAFETY: I1, I3, I4, I7: R is the running frame, made only by `enter`,
    // and AT an instruction of its function's validated code (a `Pc`'s or a
    // `Next`'s of the same call), which is what every `Handler` requires.
    unsafe { handler(hot.e, r, at) }
}
/// Run TO in the same frame.
#[inline]
pub(super) fn go<'f>(hot: Hot<'f, '_, '_>, r: Regs<'f>, to: Next<'f>) -> Value {
    call(next, hot, r, to.into_raw())
}
/// Hand PC to the kernel this typed handler is the fast path of
/// (`Inst::kernel`): its other cases. Out of line, so the hot path's
/// dispatch does not merge with this one.
#[inline(never)]
#[cold]
pub(super) fn legacy<'f>(hot: Hot<'f, '_, '_>, r: Regs<'f>, pc: Pc<'f>) -> Value {
    call(pc.inst().kernel, hot, r, pc.0)
}
/// Poll, then run TO.
#[inline]
pub(super) fn poll<'f>(hot: Hot<'f, '_, '_>, r: Regs<'f>, to: Next<'f>) -> Value {
    call(poll_resume, hot, r, to.into_raw())
}

/// A typed handler's body on what dispatch passed: its capabilities, under a
/// brand of this call alone (I7). BODY must be generic over the brand, so a
/// body that names `'static` or keeps a capability does not compile.
#[inline]
fn enter<'x, 'a, F>(e: Eng<'x, 'a>, r: *mut Value, pc: *const Inst, body: F) -> Value
where
    F: for<'f> FnOnce(Hot<'f, 'x, 'a>, Regs<'f>, Pc<'f>) -> Value,
{
    body(Hot { e, _f: PhantomData }, Regs(r, PhantomData), Pc(pc, PhantomData))
}

/// The handler of the instruction validated code ends with: reached only by
/// running off a function's end or branching to it, which lowering never
/// emits. It reads neither its frame nor its code.
pub(super) fn terminator(mut e: Eng<'_, '_>, _: *mut Value, _: *const Inst) -> Value {
    fail(&mut e, "ran past the end of lowered code")
}

/// Each typed handler as a `Handler`: an aligned entry of its own, which
/// makes the capabilities from what dispatch passes. `link_section` needs
/// `unsafe`, so the entries live here; the bodies are `threaded_hot.rs`'s.
/// `is_typed` names them all, for validation (I3).
macro_rules! handlers {
    ($($name:ident),* $(,)?) => {
        // Each on its own 64-byte line, as `threaded::control_tests` checks:
        // packed handlers once cost a fifth more cycles (dispatch-spike.md §3).
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        core::arch::global_asm!($(concat!(
            ".pushsection .text.vm2.", stringify!($name), ",\"ax\",@progbits\n.p2align 6\n.popsection"
        )),*);
        $(
            #[cfg_attr(
                all(target_os = "linux", target_arch = "x86_64"),
                // UNSAFE(link_section is unsafe in Rust; it puts each handler on a cache line of its own)
                // SAFETY: a section of this binary's own text, laid out by the
                // `global_asm!` above; it holds only this handler's code.
                unsafe(link_section = concat!(".text.vm2.", stringify!($name)))
            )]
            /// # Safety
            /// R is the running engine's current frame (`Engine::regs`) and PC an
            /// instruction of the validated code of the function running in it.
            // UNSAFE(the Handler signature: raw frame and instruction pointers, the threaded calling convention)
            pub(super) unsafe fn $name(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
                enter(e, r, pc, super::hot::$name)
            }
        )*
        /// Whether HANDLER is a typed one: only validation may install it.
        pub(super) fn is_typed(handler: Handler) -> bool {
            false $(|| std::ptr::fn_addr_eq(handler, $name as Handler))*
        }
    };
}
handlers!(
    t_mov, t_mov_wide, t_constant, t_constant_wide, t_unless, t_unless_wide, t_jump,
    t_car, t_cdr, t_add, t_sub, t_inc, t_dec, t_less, t_greater, t_equal, t_null, t_consp, t_eq,
);
