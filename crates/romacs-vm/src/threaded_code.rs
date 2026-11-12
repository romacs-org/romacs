//! Lowered code, validated once (lisp-speed.md §13.6 I3). A `Code` is made
//! only by `validate`: every register operand lies in its function's frame,
//! every argument window ends inside it, every closure site and landing
//! exists, every branch distance (`Inst::branch`, zero on other
//! instructions) lands inside the code, and the code ends with the core's
//! terminator. The handlers' raw reads rest on the first checks, the core's
//! `Pc` steps on the last two. Instructions are read through `Deref`, and
//! changed only through the methods here, which touch no operand. Lowering's
//! choice of a typed handler (`bytes`, `typed_kernel`) is here too.
use super::audited::{self, terminator};
use super::{prim, Function, Handler, Inst, Op, Primitive, Value};
use std::fmt;
use std::ops::Deref;

/// A function's lowered instructions and the terminator after them; none
/// at all before its first lowering.
pub(super) struct Code(Box<[Inst]>);

/// Why lowered code was refused: AT is the instruction's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// A register operand past the frame's WIDTH.
    Register { at: usize, register: usize, width: usize },
    /// An argument window ending past the frame's WIDTH.
    Window { at: usize, end: usize, width: usize },
    /// A closure site the function does not have.
    Site { at: usize, site: usize, sites: usize },
    /// A branch or landing to TO, outside the LEN instructions.
    Target { at: usize, to: usize, len: usize },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Refusal::Register { at, register, width } => write!(f, "instruction {at}: register {register} outside a frame of {width}"),
            Refusal::Window { at, end, width } => write!(f, "instruction {at}: argument window ending at {end} outside a frame of {width}"),
            Refusal::Site { at, site, sites } => write!(f, "instruction {at}: closure site {site} of {sites}"),
            Refusal::Target { at, to, len } => write!(f, "instruction {at}: target {to} outside {len} instructions"),
        }
    }
}

/// FUNCTION's lowered BODY as `Code`, its branch distances filled in, or why
/// not. END is any instruction: it becomes the terminator.
pub(super) fn validate(function: &Function, body: Vec<Inst>, end: Inst) -> Result<Code, Refusal> {
    for (at, inst) in body.iter().enumerate() {
        operands(function, at, &inst.op)?;
    }
    seal(body, end)
}

/// BODY with its branch distances filled in and END as its terminator, or
/// the first target outside it.
fn seal(mut body: Vec<Inst>, mut end: Inst) -> Result<Code, Refusal> {
    let len = body.len();
    for (at, inst) in body.iter_mut().enumerate() {
        let target = |to: usize| if to < len { Ok(to) } else { Err(Refusal::Target { at, to, len }) };
        inst.branch = match inst.op {
            Op::Jump(to) | Op::Unless(_, to) => {
                let to = target(to)?;
                let distance = if to >= at { i32::try_from(to - at) } else { i32::try_from(at - to).map(|d| -d) };
                distance.map_err(|_| Refusal::Target { at, to, len })?
            }
            Op::Catch(_, _, to) | Op::Condition(_, _, to) | Op::Protect(_, to, _) => {
                target(to)?;
                0
            }
            _ => 0,
        };
    }
    end.handler = terminator;
    end.branch = 0;
    body.push(end);
    Ok(Code(body.into_boxed_slice()))
}

/// Whether OP, instruction AT of FUNCTION, names only registers and sites
/// FUNCTION has.
fn operands(function: &Function, at: usize, op: &Op) -> Result<(), Refusal> {
    let width = function.regs;
    let window = |start: usize, count: usize| match start.checked_add(count) {
        Some(end) if end <= width => Ok(()),
        _ => Err(Refusal::Window { at, end: start.saturating_add(count), width }),
    };
    let mut registers = *op;
    match *op {
        Op::Call(_, _, start, count) | Op::Direct(_, _, _, start, count) | Op::Sealed(_, _, start, count) => {
            window(start, count)?;
            // The window is checked whole; its first register may be its end.
            match &mut registers {
                Op::Call(_, _, s, _) | Op::Direct(_, _, _, s, _) | Op::Sealed(_, _, s, _) => *s = 0,
                _ => {}
            }
        }
        Op::Closure(_, _, site) if site >= function.sites.len() => {
            return Err(Refusal::Site { at, site, sites: function.sites.len() });
        }
        Op::Closure(_, _, site) => {
            for &register in &function.sites[site] {
                window(register, 1)?;
            }
        }
        _ => {}
    }
    let mut outside = None;
    registers.map_registers(&mut |register| {
        if register >= width && outside.is_none() {
            outside = Some(register);
        }
        register
    });
    match outside {
        Some(register) => Err(Refusal::Register { at, register, width }),
        None => Ok(()),
    }
}

/// Lowering's choice of handler: a typed one where the operands fit a byte.
/// An operand `Inst` keeps in 32 bits: a register, a count or a site.
pub(super) fn narrow(operand: usize) -> u32 {
    u32::try_from(operand).expect("an operand fits 32 bits")
}
/// OP's handler on a frame whose operands A, B and D fit a byte, that is
/// TYPED, else WIDE, either with the bytes `Inst` keeps for it.
pub(super) fn bytes(i: &mut Inst, typed: Handler, wide: Handler) {
    match (u8::try_from(i.a), u8::try_from(i.b), u8::try_from(i.d)) {
        (Ok(a), Ok(b), Ok(d)) => {
            (i.a8, i.b8, i.d8) = (a, b, d);
            i.handler = typed;
        }
        _ => i.handler = wide,
    }
}
/// The typed fast path (`threaded_hot.rs`) of `kernel(TARGET, ARGC)`, if it
/// has one: X at D and Y at D+1 in a window.
pub(super) fn typed_kernel(target: Value, argc: usize) -> Option<Handler> {
    if let Some(d) = prim::generated(target) {
        return matches!((d.name, argc), (">", 2)).then_some(audited::t_greater as Handler);
    }
    Some(match (prim::legacy(target)?, argc) {
        (Primitive::Car, 1) => audited::t_car,
        (Primitive::Cdr, 1) => audited::t_cdr,
        (Primitive::Null, 1) => audited::t_null,
        (Primitive::Eq, 2) => audited::t_eq,
        (Primitive::Add, 2) => audited::t_add,
        (Primitive::Sub, 2) => audited::t_sub,
        (Primitive::Inc, 1) => audited::t_inc,
        (Primitive::Dec, 1) => audited::t_dec,
        (Primitive::Less, 2) => audited::t_less,
        (Primitive::Equal, 2) => audited::t_equal,
        (Primitive::Consp, 1) => audited::t_consp,
        _ => return None,
    })
}
/// A sealed kernel call takes TYPED when its operands fit a byte, the kernel
/// it was given becoming the typed handler's other cases (`Inst::kernel`).
pub(super) fn kernel_bytes(i: &mut Inst, typed: Handler) {
    let second = i.d.checked_add(1).map(u8::try_from);
    if let (Ok(a), Ok(d), Some(Ok(b))) = (u8::try_from(i.a), u8::try_from(i.d), second) {
        (i.a8, i.d8, i.b8) = (a, d, b);
        i.kernel = i.handler;
        i.handler = typed;
    }
}

impl Code {
    /// A test's hand-made BODY, its operands unchecked: there is no function.
    #[cfg(test)]
    pub(super) fn for_test(body: Vec<Inst>, end: Inst) -> Code {
        seal(body, end).expect("a test's branches land in its code")
    }
    /// No code: its function was not lowered yet, or was released.
    pub(super) fn none() -> Code {
        Code(Box::default())
    }
    /// Whether this is `none`.
    pub(super) fn is_none(&self) -> bool {
        self.0.is_empty()
    }
    /// The first instruction, the terminator's if the body is empty.
    #[inline]
    pub(super) fn entry(&self) -> *const Inst {
        self.0.as_ptr()
    }
    /// Every Value the instructions hold, in place, for the collector.
    pub(super) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        let body = self.0.len().saturating_sub(1);
        for inst in &mut self.0[..body] {
            inst.visit_values(visit);
        }
    }
    /// Every instruction's entry-cache site, in place, to renumber them.
    pub(super) fn sites_mut(&mut self) -> impl Iterator<Item = &mut usize> {
        let body = self.0.len().saturating_sub(1);
        self.0[..body].iter_mut().map(|inst| &mut inst.site)
    }
}

impl Deref for Code {
    type Target = [Inst];
    /// The instructions, the terminator left out.
    fn deref(&self) -> &[Inst] {
        &self.0[..self.0.len().saturating_sub(1)]
    }
}

#[cfg(test)]
mod tests {
    use super::super::{lower, Vm};
    use super::*;

    /// The function `defun` gave NAME, lowered as `lower_function` lowers it.
    fn function(vm: &mut Vm, source: &str, name: &str) -> (Function, Vec<Inst>, Inst) {
        vm.eval_source_raw(source).unwrap();
        let closure = vm.slots[vm.names[name]].function;
        let id = vm.closures[usize::try_from(closure.payload()).unwrap()].function;
        vm.materialize_image_function(id).unwrap();
        let function = Function::clone(&vm.functions[id]);
        let body = function.code.iter().map(|&op| lower(vm, op)).collect();
        (function, body, lower(vm, Op::ResumeExit))
    }

    #[test]
    fn validated_code_holds_its_branches_and_ends_in_the_terminator() {
        let mut vm = Vm::new();
        let (f, body, end) = function(&mut vm, "(defun st-loop (x) (while x (setq x (cdr x))) 7)", "st-loop");
        let code = validate(&f, body, end).unwrap();
        assert_eq!(code.len(), f.code.len());
        let mut branches = 0;
        for (at, inst) in code.iter().enumerate() {
            match inst.op {
                Op::Jump(to) | Op::Unless(_, to) => {
                    branches += 1;
                    assert_eq!(at.checked_add_signed(inst.branch as isize), Some(to));
                }
                _ => assert_eq!(inst.branch, 0, "a branch distance off a branch"),
            }
        }
        assert!(branches >= 2, "the loop branches back and out");
        let terminator: Handler = terminator;
        assert!(std::ptr::fn_addr_eq(code.0[code.len()].handler, terminator));
    }

    #[test]
    fn code_naming_what_its_function_lacks_is_refused() {
        let mut vm = Vm::new();
        let (f, body, end) = function(&mut vm, "(defun st-plain (x y) (if x (cons x y) y))", "st-plain");
        let width = f.regs;
        let len = body.len();
        let refusal = |op: Op| {
            let mut changed = body.clone();
            changed[0] = lower(&vm, op);
            validate(&f, changed, end).err()
        };
        assert_eq!(refusal(Op::Move(0, width)), Some(Refusal::Register { at: 0, register: width, width }));
        assert_eq!(refusal(Op::Return(width + 3)), Some(Refusal::Register { at: 0, register: width + 3, width }));
        assert_eq!(refusal(Op::Call(0, 0, width - 1, 2)), Some(Refusal::Window { at: 0, end: width + 1, width }));
        assert_eq!(refusal(Op::Closure(0, 0, f.sites.len())), Some(Refusal::Site { at: 0, site: f.sites.len(), sites: f.sites.len() }));
        assert_eq!(refusal(Op::Jump(len)), Some(Refusal::Target { at: 0, to: len, len }));
        assert_eq!(refusal(Op::Unless(0, len + 9)), Some(Refusal::Target { at: 0, to: len + 9, len }));
        assert_eq!(refusal(Op::Catch(0, 0, len)), Some(Refusal::Target { at: 0, to: len, len }));
        assert_eq!(refusal(Op::Call(0, 0, width, 0)), None, "an empty window may start at the frame's end");
    }

    #[test]
    fn running_off_the_end_signals() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun st-cut (x) (setq x (car x)) x)").unwrap();
        let closure = vm.slots[vm.names["st-cut"]].function;
        let id = vm.closures[usize::try_from(closure.payload()).unwrap()].function;
        vm.materialize_image_function(id).unwrap();
        assert!(matches!(std::sync::Arc::make_mut(&mut vm.functions[id]).code.pop(), Some(Op::Return(_))));
        let error = vm.eval_source_raw("(st-cut '(1))").unwrap_err();
        assert!(error.contains("ran past the end of lowered code"), "{error}");
    }

    /// A frame wider than a byte runs the checked `_wide` handlers for the
    /// operands past 255, and the typed ones below it, to the same result.
    #[test]
    fn a_frame_wider_than_a_byte_runs_its_checked_handlers() {
        let mut vm = Vm::new();
        let bindings: Vec<String> = (0..300).map(|i| format!("(v{i} {i})")).collect();
        vm.eval_source_raw(&format!(
            "(defun st-wide (flag) (let* ({}) (setq v299 (if flag v298 nil)) (if v299 (list v0 v299) (list v1 'none))))",
            bindings.join(" ")
        ))
        .unwrap();
        for (call, expected) in [("(st-wide t)", "(0 298)"), ("(st-wide nil)", "(1 none)")] {
            let value = vm.eval_source_raw(call).unwrap();
            assert_eq!(vm.display(value).unwrap(), expected);
        }
        let closure = vm.slots[vm.names["st-wide"]].function;
        let id = vm.closures[usize::try_from(closure.payload()).unwrap()].function;
        let code = &vm.execution.code[id];
        let has = |handler: Handler| code.iter().any(|i| std::ptr::fn_addr_eq(i.handler, handler));
        for (name, handler) in [
            ("constant", audited::t_constant as Handler), ("constant_wide", audited::t_constant_wide),
            ("mov_wide", audited::t_mov_wide), ("unless_wide", audited::t_unless_wide),
        ] {
            assert!(has(handler), "no {name} in a 300-register frame");
        }
    }
}
