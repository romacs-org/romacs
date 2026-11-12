//! The audited core of the threaded engine (lisp-speed.md §13.6): every
//! `unsafe` the typed handlers (`threaded_hot.rs`, which forbids it) rely
//! on, behind capabilities they cannot forge. Each site names the invariant
//! it rests on:
//!
//! - I1, the file: `WINDOW` (256) initialised slots lie above every window
//!   (`frame_room`), so a 257-slot view from the base of a frame one register
//!   wide or more stays in its
//!   file. The file moves only by growth, which no typed handler reaches.
//! - I2 and I5, windows: a view borrows its `Regs` and the engine
//!   exclusively, so nothing else reads or writes the file while it lives.
//! - I3, validated code: a `Code` is made only by `threaded_code.rs`, which
//!   holds every register operand inside its frame, every branch inside it and ends it with `terminator`, whose
//!   handler is here, so no body is handed the terminator's `Pc`.
//! - I4, instruction pointers: a `Pc` points into a `Code` that outlives the
//!   handler holding it: code is replaced or cleared only while no engine
//!   runs it (`Program::retire_code`, `lower_function`).
//! - I6, the files: `Registers` alone changes a file's allocation.
//!
//! Not guaranteed by any type: that the engine's modules make no second
//! `Vm` to swap a running one's files with; `tools/lib/smells.py` holds that
//! (`vm-replacement`).
use super::{fail, next, poll_resume, Eng, Handler, Inst, Value};

/// The running frame's registers: one per engine, moved along dispatch, made
/// only here, and not `Copy`, so none outlives the handler it was given to.
#[repr(transparent)]
pub(super) struct Regs(*mut Value);

/// The running instruction. Operands are read by value, so no reference into
/// the code outlives a read, and a collection may rewrite the code's Values.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub(super) struct Pc(*const Inst);

/// A register operand of a frame wider than a byte, which validation
/// proved lies inside the frame: made only from a `Pc`'s operand.
#[derive(Clone, Copy)]
pub(super) struct Wide(usize);

/// Where an instruction goes. It can only be dispatched to, so a handler
/// takes one step at most and reads nothing through it.
#[repr(transparent)]
pub(super) struct Next(*const Inst);

impl Regs {
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
    pub(super) fn window<'w>(&'w mut self, _engine: &'w mut Eng<'_, '_>) -> &'w mut [Value; 257] {
        // SAFETY: I3: a typed handler's operands are validated below its
        // frame's width, so the frame is one register wide or more, and I1
        // puts 256 initialised slots past it; I5 makes this the one reference
        // to them while it lives.
        unsafe { &mut *self.0.cast::<[Value; 257]>() }
    }
    /// Register AT of a frame wider than a byte: what its `Wide` operand
    /// names, read or written through the slot. Borrows as `window` does.
    #[inline]
    pub(super) fn slot<'w>(&'w mut self, _engine: &'w mut Eng<'_, '_>, at: Wide) -> &'w mut Value {
        // SAFETY: I3: `code::validate` proved AT below the frame's width, and
        // I1 puts that width and 256 initialised slots past the base; I5
        // makes this the one reference to it while it lives.
        unsafe { &mut *self.0.add(at.0) }
    }
}

impl Next {
    /// The instruction, the capability spent: what dispatch hands on.
    #[inline]
    fn into_raw(self) -> *const Inst {
        self.0
    }
}

impl Pc {
    #[inline]
    fn inst(&self) -> &Inst {
        // SAFETY: I3 and I4: a Pc is made from an instruction of validated
        // code that outlives it, and this reference never leaves this module.
        unsafe { &*self.0 }
    }
    /// The byte operands, for typed handlers (`threaded::bytes`).
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
    pub(super) fn wide_a(self) -> Wide {
        Wide(self.inst().a)
    }
    #[inline]
    pub(super) fn wide_b(self) -> Wide {
        Wide(self.inst().b)
    }
    #[inline]
    pub(super) fn value(self) -> Value {
        self.inst().value
    }
    #[inline]
    fn step(self, distance: isize) -> Next {
        // SAFETY: I3: every instruction's branch distance lands in its code,
        // and 1 does too, as validated code ends in the terminator and no
        // typed handler is handed the terminator's Pc.
        Next(unsafe { self.0.offset(distance) })
    }
    /// The following instruction.
    #[inline]
    pub(super) fn next(self) -> Next {
        self.step(1)
    }
    /// This instruction's branch target (`Inst::branch`).
    #[inline]
    pub(super) fn branch(self) -> Next {
        self.step(self.inst().branch as isize)
    }
    /// Whether TO runs backward from here: a loop's poll.
    #[inline]
    pub(super) fn is_back(self, to: &Next) -> bool {
        to.0 <= self.0
    }
}

/// HANDLER on the frame R and the instruction AT: every way out of a typed
/// handler is this one call.
#[inline]
fn call(handler: Handler, e: Eng<'_, '_>, r: Regs, at: *const Inst) -> Value {
    let r = r.into_raw();
    // SAFETY: I1, I3, I4: R is the running frame, made only by the entries
    // below, and AT an instruction of its function's validated code (a
    // `Pc`'s or a `Next`'s), which is what every `Handler` requires.
    unsafe { handler(e, r, at) }
}
/// Run TO in the same frame.
#[inline]
pub(super) fn go(e: Eng<'_, '_>, r: Regs, to: Next) -> Value {
    call(next, e, r, to.into_raw())
}
/// Hand PC to the kernel this typed handler is the fast path of
/// (`Inst::kernel`): its other cases. Out of line, so the hot path's
/// dispatch does not merge with this one.
#[inline(never)]
#[cold]
pub(super) fn legacy(e: Eng<'_, '_>, r: Regs, pc: Pc) -> Value {
    call(pc.inst().kernel, e, r, pc.0)
}
/// Poll, then run TO.
#[inline]
pub(super) fn poll(e: Eng<'_, '_>, r: Regs, to: Next) -> Value {
    call(poll_resume, e, r, to.into_raw())
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
                // SAFETY: a section of this binary's own text, laid out by the
                // `global_asm!` above; it holds only this handler's code.
                unsafe(link_section = concat!(".text.vm2.", stringify!($name)))
            )]
            /// # Safety
            /// R is the running engine's current frame (`Engine::regs`) and PC an
            /// instruction of the validated code of the function running in it.
            pub(super) unsafe fn $name(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
                super::hot::$name(e, Regs(r), Pc(pc))
            }
        )*
    };
}
handlers!(
    t_mov, t_mov_wide, t_constant, t_constant_wide, t_unless, t_unless_wide, t_jump,
    t_car, t_cdr, t_add, t_sub, t_inc, t_dec, t_less, t_greater, t_equal, t_null, t_consp, t_eq,
);
