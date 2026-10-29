//! Private execution lowering. The public readable IR stays unchanged.
//!
//! Safety boundary: only the Rust compiler constructs Op; lowering preserves its
//! register/branch bounds. Register storage grows at activation boundaries;
//! every live register pointer is rebased before dispatch resumes. Code arrays
//! and functions keep address-stable buffers; GC forwards their Value operands
//! only at wait points. No pointer escapes the VM's retained Program lifetime.
use super::*;
#[path = "threaded_signal.rs"]
mod signal_handlers;
pub(super) use signal_handlers::SignalHandler;
use signal_handlers::{prepare_signal, release_control};
#[cfg(test)]
#[path = "error_signal_tests.rs"]
mod error_signal_tests;
#[cfg(test)]
#[path = "inline_tests.rs"]
mod inline_tests;
#[path = "traversal.rs"]
mod traversal;
#[cfg(test)]
#[path = "global_read_tests.rs"]
mod global_read_tests;
#[cfg(test)]
#[path = "world_epoch_tests.rs"]
mod world_epoch_tests;
#[cfg(test)]
mod control_tests {
    use super::*;

    #[test]
    fn frame_is_48_bytes_without_drop() {
        assert_eq!(std::mem::size_of::<Frame>(), 48);
        assert!(!std::mem::needs_drop::<Frame>());
    }

    #[test]
    fn entry_cache_is_48_bytes_without_drop() {
        assert_eq!(std::mem::size_of::<EntryCache>(), 48);
        assert!(!std::mem::needs_drop::<EntryCache>());
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn hot_handlers_are_fetch_aligned() {
        for handler in [mov, jump, unless, mul, add, sub, inc, dec, less, equal] {
            assert_eq!((handler as usize) % 64, 0);
        }
    }

    #[test]
    fn inline_side_table_tracks_release_and_compaction() {
        let vm = Vm::new();
        let mut program = Program::default();
        let entry = EntryCache {
            world_version: 1,
            target: Value::UNBOUND,
            function: 0,
            entry: std::ptr::null(),
            simple_arity: 0,
            primitive: None,
        };
        let mut instructions = Vec::new();
        for site in 0..3 {
            let mut inst = lower(&vm, Op::Return(0));
            inst.site = site;
            instructions.push(inst);
            program.entries.push(entry);
            program.inline_bodies.push(Some(InlineBody {
                primitive: None,
                left: InlineOperand::Integer(10 + site as i64),
                right: InlineOperand::Nil,
            }));
        }
        program.code.push(instructions[..2].to_vec());
        program.code.push(instructions[2..].to_vec());
        program.release_entry(0);
        assert!(program.inline_bodies[..2].iter().all(Option::is_none));
        assert_eq!(program.free_entries, [0, 1]);
        program.retire_code(&[false, true]);
        assert_eq!(program.entries.len(), 1);
        assert_eq!(program.inline_bodies.len(), 1);
        assert_eq!(program.code[1][0].site, 0);
        assert!(program.free_entries.is_empty());
        // Immediate-only descriptor evaluation never dereferences the argument pointer.
        assert_eq!(
            unsafe { program.inline_bodies[0].unwrap().evaluate(std::ptr::null()) },
            Some(Value::integer(12).unwrap())
        );
    }

    fn empty_engine(vm: &mut Vm) -> Engine<'_> {
        Engine {
            vm,
            code: vec![],
            rest: RestStack {
                values: Vec::new(),
                activations: vec![(0, None)],
            },
            frames: vec![],
            call_marks: vec![None],
            native_mark: None,
            apply_function: None,
            apply_mark: None,
            stack_used: 0,
            apply_scratch: Vec::new(),
            depth_base: 0,
            regs: std::ptr::null_mut(),
            function: 0,
            dynamic: 0,
            error: None,
            fatal_image_failure: false,
            escape: None,
            escape_hooked: false,
            failed_eval_depth: None,
        native_floor: 0,
            propagate: false,
            controls: vec![],
            controls_base: 0,
            budget: 0,
            entries: vec![],
            inline_bodies: vec![],
            free_entries: vec![],
            continuations: vec![],
            #[cfg(debug_assertions)]
            resume: None,
        }
    }

    #[test]
    fn malformed_callee_region_ops_cannot_consume_caller_controls() {
        for (op, kind, diagnostic) in [
            (
                Op::PopHandlers(1),
                ControlKind::Catch(Value::TRUE),
                "invalid image handler depth",
            ),
            (
                Op::LeaveProtect(0),
                ControlKind::Protect { end: 0 },
                "invalid image protect depth",
            ),
            (
                Op::ResumeExit,
                ControlKind::Resume {
                    exit: Escape::Normal(Value::TRUE),
                    diagnostic: None,
                },
                "invalid image resume depth",
            ),
        ] {
            let mut vm = Vm::new();
            let instructions = [lower(&vm, op), lower(&vm, Op::Return(0))];
            let mut regs = [Value::NIL];
            let mut e = empty_engine(&mut vm);
            e.controls.push(Control {
                kind,
                dest: 0,
                target: 0,
                dynamic: 0,
                registry: None,
            });
            e.controls_base = 1;
            // SAFETY: both instructions and the single addressed register are live.
            unsafe {
                cold(&mut e, regs.as_mut_ptr(), instructions.as_ptr());
            }
            assert_eq!(e.error.as_deref(), Some(diagnostic));
            assert_eq!(e.controls.len(), 1, "callee consumed caller control");
        }
    }

    #[test]
    fn normal_escape_cannot_cross_activation_floor() {
        let mut vm = Vm::new();
        let code = vec![lower(&vm, Op::Return(0))];
        let mut regs = [Value::NIL];
        let mut e = empty_engine(&mut vm);
        e.code.push(code);
        e.regs = regs.as_mut_ptr();
        e.controls.push(Control {
            kind: ControlKind::Protect { end: 0 },
            dest: 0,
            target: 0,
            dynamic: 0,
            registry: None,
        });
        e.controls_base = 1;
        e.escape = Some(Escape::Normal(Value::TRUE));
        assert!(route_escape(&mut e).is_none());
        assert_eq!(e.error.as_deref(), Some("invalid image protect depth"));
        assert_eq!(e.controls.len(), 1);
    }

    #[test]
    fn suspended_caller_regions_are_exclusive_forwarded_roots() {
        let mut vm = Vm::new();
        for _ in 0..20 {
            vm.conses.push((Value::NIL, Value::NIL));
        }
        let tag = vm.list_value(&[Value::integer(17).unwrap()]);
        let payload = vm.list_value(&[Value::integer(42).unwrap()]);
        let conditions = vm.list_value(&[Value::integer(73).unwrap()]);
        let mut e = Engine {
            vm: &mut vm,
            code: vec![],
            rest: RestStack {
                values: Vec::new(),
                activations: vec![(0, None)],
            },
            frames: vec![Frame {
                pc: std::ptr::null(),
                regs: std::ptr::null_mut(),
                dest: 0,
                function: 0,
                dynamic: 0,
                controls_base: 0,
            }],
            call_marks: vec![None, None],
            native_mark: None,
            apply_function: None,
            apply_mark: None,
            stack_used: 0,
            apply_scratch: Vec::new(),
            depth_base: 0,
            regs: std::ptr::null_mut(),
            function: 0,
            dynamic: 0,
            error: None,
            fatal_image_failure: false,
            escape: None,
            escape_hooked: false,
            failed_eval_depth: None,
        native_floor: 0,
            propagate: false,
            controls: vec![Control {
                kind: ControlKind::Resume {
                    exit: Escape::Throw(tag, payload),
                    diagnostic: None,
                },
                dest: 0,
                target: 0,
                dynamic: 0,
                registry: None,
            }],
            controls_base: 4,
            budget: 0,
            entries: vec![],
            inline_bodies: vec![],
            free_entries: vec![],
            continuations: vec![],
            #[cfg(debug_assertions)]
            resume: None,
        };
        e.controls.extend([
            Control {
                kind: ControlKind::Catch(tag),
                dest: 0,
                target: 0,
                dynamic: 0,
                registry: None,
            },
            Control {
                kind: ControlKind::Condition(conditions),
                dest: 0,
                target: 0,
                dynamic: 0,
                registry: None,
            },
            Control {
                kind: ControlKind::Resume {
                    exit: Escape::Normal(payload),
                    diagnostic: None,
                },
                dest: 0,
                target: 0,
                dynamic: 0,
                registry: None,
            },
        ]);
        e.controls.push(Control {
            kind: ControlKind::Resume {
                exit: Escape::Signal(conditions, payload),
                diagnostic: None,
            },
            dest: 0,
            target: 0,
            dynamic: 0,
            registry: None,
        });
        let mut roots = vec![];
        e.visit_values(&mut |v| roots.push(*v));
        assert_eq!(
            roots.len(),
            7,
            "each live handler Value must be traced once"
        );
        e.vm.collect(&mut roots);
        let mut forwarded = roots.into_iter();
        e.visit_values(&mut |v| *v = forwarded.next().unwrap());
        let ControlKind::Resume {
            exit: Escape::Throw(new_tag, new_payload),
            ..
        } = &e.controls[0].kind
        else {
            panic!("lost suspended exit")
        };
        assert_ne!(*new_tag, tag);
        assert_ne!(*new_payload, payload);
        assert_eq!(e.vm.display(*new_tag).unwrap(), "(17)");
        assert_eq!(e.vm.display(*new_payload).unwrap(), "(42)");
        assert_eq!(e.vm.heap_stats().collections, 1);
        assert_eq!(e.vm.heap_stats().reclaimed, 20);
        let mut after = Vec::new();
        e.visit_values(&mut |v| after.push(*v));
        assert_eq!(after.len(), 7);
        for (value, expected) in after
            .iter()
            .zip(["(17)", "(42)", "(17)", "(73)", "(42)", "(73)", "(42)"])
        {
            assert_eq!(e.vm.display(*value).unwrap(), expected);
        }
        for value in after {
            assert!(
                ![tag, payload, conditions].contains(&value),
                "handler Value did not move"
            );
        }
    }
}

// Keep these tiny dispatch bodies within aligned fetch blocks. A layout-only
// control restored the call-free loop without changing its retired instructions.
// The empty ELF sections set alignment without executing padding in the handler.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
core::arch::global_asm!(
    ".pushsection .text.vm2.mul,\"ax\",@progbits",
    ".p2align 6",
    ".popsection",
    ".pushsection .text.vm2.sub,\"ax\",@progbits",
    ".p2align 6",
    ".popsection",
    ".pushsection .text.vm2.dec,\"ax\",@progbits",
    ".p2align 6",
    ".popsection",
    ".pushsection .text.vm2.equal,\"ax\",@progbits",
    ".p2align 6",
    ".popsection",
    ".pushsection .text.vm2.mov,\"ax\",@progbits",
    ".p2align 6",
    ".popsection",
    ".pushsection .text.vm2.add,\"ax\",@progbits",
    ".p2align 6",
    ".popsection",
    ".pushsection .text.vm2.inc,\"ax\",@progbits",
    ".p2align 6",
    ".popsection",
    ".pushsection .text.vm2.less,\"ax\",@progbits",
    ".p2align 6",
    ".popsection",
    ".pushsection .text.vm2.jump,\"ax\",@progbits",
    ".p2align 6",
    ".popsection",
    ".pushsection .text.vm2.unless,\"ax\",@progbits",
    ".p2align 6",
    ".popsection",
);

type Handler = unsafe fn(&mut Engine<'_>, *mut Value, *const Inst) -> Value;
#[derive(Clone, Copy)]
struct Inst {
    handler: Handler,
    observed_handler: Handler,
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    value: Value,
    op: Op,
    site: usize,
    prepared_handler: Handler,
}
type PrimitiveCall = fn(&mut Vm, Primitive, usize, usize) -> Result<Value>;
#[derive(Clone, Copy)]
struct EntryCache {
    // Nonzero only for a direct, capture-free, exact-arity closure.
    world_version: u64,
    target: Value,
    function: usize,
    entry: *const Inst,
    simple_arity: usize,
    primitive: Option<PrimitiveCall>,
}

// A deliberately narrow expression language, not copied IR. Operands contain
// only argument indices or immediate scalars, never an untraced heap Value.
#[derive(Clone, Copy)]
enum InlineOperand {
    Argument(usize),
    Integer(i64),
    Nil,
    True,
}
#[derive(Clone, Copy)]
struct InlineBody {
    primitive: Option<Primitive>,
    left: InlineOperand,
    right: InlineOperand,
}

// Recognize at most three setup/primitive ops and a return. One pure primitive
// is enough for the call-floor workload. Reject all other effects/regions and
// all intermediate results; this keeps bailout completely side-effect free.
#[cold]
#[inline(never)]
fn inline_body(vm: &Vm, function: usize) -> Option<InlineBody> {
    let f = &vm.functions[function];
    if f.captures != 0 || f.rest || f.required != f.arity || f.code.len() > 8 {
        return None;
    }
    let mut operands = vec![None; f.regs];
    for (n, operand) in operands.iter_mut().take(f.arity).enumerate() {
        *operand = Some(InlineOperand::Argument(n));
    }
    let mut body = None;
    let mut result_reg = None;
    let mut resolved = None;
    for (index, op) in f.code.iter().enumerate() {
        match *op {
            Op::Const(dst, value) => {
                operands[dst] = Some(if let Some(n) = value.integer_value() {
                    InlineOperand::Integer(n)
                } else if value == Value::NIL {
                    InlineOperand::Nil
                } else if value == Value::TRUE {
                    InlineOperand::True
                } else {
                    return None;
                });
                if result_reg == Some(dst) {
                    return None;
                }
            }
            Op::Move(dst, src) => {
                if result_reg == Some(src) {
                    result_reg = Some(dst);
                } else {
                    operands[dst] = operands[src];
                    if result_reg == Some(dst) {
                        return None;
                    }
                }
            }
            Op::ResolveHead(dst, slot) if resolved.is_none() && body.is_none() => {
                resolved = Some((dst, slot));
            }
            Op::Direct(dst, slot, _, start, argc)
            | Op::Prepared(dst, _, slot, _, start, argc) if body.is_none() => {
                if let Op::Prepared(_, snapshot, _, _, _, _) = *op {
                    if resolved != Some((snapshot, slot)) { return None; }
                }
                let target = vm.slots[slot].function;
                let primitive = prim::legacy(target)?;
                let arity = match primitive {
                    Primitive::Inc | Primitive::Dec | Primitive::Null => 1,
                    Primitive::Add
                    | Primitive::Sub
                    | Primitive::Mul
                    | Primitive::Less
                    | Primitive::Equal
                    | Primitive::Eq => 2,
                    _ => return None,
                };
                if argc != arity || vm.macros.contains(&slot) {
                    return None;
                }
                body = Some(InlineBody {
                    primitive: Some(primitive),
                    left: operands[start]?,
                    right: if argc == 2 {
                        operands[start + 1]?
                    } else {
                        InlineOperand::Nil
                    },
                });
                result_reg = Some(dst);
                operands[dst] = None;
            }
            Op::Return(src) if index + 1 == f.code.len() => {
                return if let Some(body) = body {
                    (result_reg == Some(src)).then_some(body)
                } else {
                    Some(InlineBody {
                        primitive: None,
                        left: operands[src]?,
                        right: InlineOperand::Nil,
                    })
                };
            }
            _ => return None,
        }
    }
    None
}

impl InlineBody {
    // No allocation, collection, Lisp call or caller-register write here. Type
    // and overflow misses restart the unchanged generic activation instead of
    // inventing a second numeric/error implementation.
    // Keep numeric fallbacks and their temporary error values outside the
    // dispatch handler, so LLVM can emit a sibling call to the next opcode.
    #[inline(never)]
    unsafe fn evaluate(&self, args: *const Value) -> Option<Value> {
        let read = |operand| match operand {
            InlineOperand::Argument(n) => unsafe { *args.add(n) },
            InlineOperand::Integer(n) => Value::integer(n).unwrap(),
            InlineOperand::Nil => Value::NIL,
            InlineOperand::True => Value::TRUE,
        };
        let x = read(self.left);
        let Some(p) = self.primitive else {
            return Some(x);
        };
        let y = read(self.right);
        if p == Primitive::Null {
            return Some(if x == Value::NIL {
                Value::TRUE
            } else {
                Value::NIL
            });
        }
        if p == Primitive::Eq {
            return Some(if x == y { Value::TRUE } else { Value::NIL });
        }
        let a = x.integer_value()?;
        let b = if matches!(p, Primitive::Inc | Primitive::Dec) {
            1
        } else {
            y.integer_value()?
        };
        match p {
            Primitive::Inc | Primitive::Add => {
                a.checked_add(b).and_then(|n| Value::integer(n).ok())
            }
            Primitive::Dec | Primitive::Sub => {
                a.checked_sub(b).and_then(|n| Value::integer(n).ok())
            }
            Primitive::Mul => a.checked_mul(b).and_then(|n| Value::integer(n).ok()),
            Primitive::Less => Some(if a < b { Value::TRUE } else { Value::NIL }),
            Primitive::Equal => Some(if a == b { Value::TRUE } else { Value::NIL }),
            _ => unreachable!("inline recognizer primitive whitelist"),
        }
    }
}
/// Private lowering is appended once and GC-forwarded in place. Retain it across externally
/// submitted forms instead of relowering the complete retained world per eval.
#[derive(Default)]
pub(super) struct Program {
    code: Vec<Vec<Inst>>,
    entries: Vec<EntryCache>,
    inline_bodies: Vec<Option<InlineBody>>,
    free_entries: Vec<usize>,
    // Idle storage contains only NIL and is not part of the GC root set.
    registers: Vec<Value>,
    // Empty between executions; reuse argument-stack capacity like registers.
    rest: RestStack,
}
impl Program {
    #[cfg(test)]
    pub(super) fn rest_stack_capacity(&self) -> (usize, usize) {
        assert!(self.rest.values.is_empty());
        assert!(self.rest.activations.is_empty());
        (
            self.rest.values.capacity(),
            self.rest.activations.capacity(),
        )
    }
    #[cfg(test)]
    pub(super) fn code_is_empty(&self) -> bool {
        self.code.iter().all(Vec::is_empty) && self.entries.is_empty()
    }
    pub(super) fn trace_function(
        &self,
        id: usize,
        values: &mut Vec<Value>,
        functions: &mut Vec<usize>,
    ) {
        let Some(code) = self.code.get(id) else {
            return;
        };
        for inst in code {
            values.push(inst.value);
            let mut op = inst.op;
            op.visit_values(&mut |v| values.push(*v));
            let entry = self.entries[inst.site];
            values.push(entry.target);
            if !entry.entry.is_null() {
                functions.push(entry.function);
            }
        }
    }
    pub(super) fn release_entry(&mut self, id: usize) {
        if let Some(code) = self.code.get_mut(id) {
            for inst in std::mem::take(code) {
                self.inline_bodies[inst.site] = None;
                self.entries[inst.site].world_version = 0;
                self.entries[inst.site].target = Value::UNBOUND;
                self.entries[inst.site].entry = std::ptr::null();
                self.free_entries.push(inst.site);
            }
        }
    }
    pub(super) fn retire_code(&mut self, marks: &[bool]) {
        let old = std::mem::take(&mut self.entries);
        let old_inline = std::mem::take(&mut self.inline_bodies);
        self.free_entries.clear();
        for (id, code) in self.code.iter_mut().enumerate() {
            if !marks[id] {
                *code = Vec::new();
            } else {
                for inst in code {
                    let entry = old[inst.site];
                    self.inline_bodies.push(old_inline[inst.site]);
                    inst.site = self.entries.len();
                    self.entries.push(entry);
                }
            }
        }
    }
    pub(super) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit_code(&mut self.code, visit);
        for entry in &mut self.entries {
            visit(&mut entry.target);
        }
    }
}
/// A suspended engine's lowered code and entry caches, visited in place by
/// the collector while a nested call runs (`Vm::suspended_code`): copying
/// them into the root list cost a pass over all code per `read' call.
pub(super) struct SuspendedCode {
    code: *mut Vec<Vec<Inst>>,
    entries: *mut Vec<EntryCache>,
}
impl SuspendedCode {
    /// SAFETY: the engine owning both vectors stays suspended, neither moved
    /// nor touched, until this reference is popped.
    pub(super) unsafe fn visit_values(&self, visit: &mut impl FnMut(&mut Value)) {
        unsafe {
            visit_code(&mut *self.code, visit);
            for entry in &mut *self.entries {
                visit(&mut entry.target);
            }
        }
    }
}
fn visit_code(code: &mut [Vec<Inst>], visit: &mut impl FnMut(&mut Value)) {
    for inst in code.iter_mut().flatten() {
        visit(&mut inst.value);
        inst.op.visit_values(visit);
    }
}
#[derive(Clone, Copy)]
struct Frame {
    pc: *const Inst,
    regs: *mut Value,
    dest: usize,
    function: usize,
    dynamic: usize,
    // Suspended caller's floor; the callee's floor is the shared stack length at entry.
    controls_base: usize,
}
pub(super) enum Escape {
    Normal(Value),
    Throw(Value, Value),
    Signal(Value, Value),
    Error { condition: Value, object: Value, conditions: Value, target: Option<usize> },
}
impl Escape {
    pub(super) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        match self {
            Self::Normal(v) => visit(v),
            Self::Throw(a, b) | Self::Signal(a, b) => { visit(a); visit(b); }
            Self::Error { condition, object, conditions, .. } => {
                visit(condition); visit(object); visit(conditions);
            }
        }
    }
}
enum ControlKind {
    HandlerBind { mark: usize },
    Catch(Value),
    Condition(Value),
    Protect {
        end: usize,
    },
    Resume {
        exit: Escape,
        diagnostic: Option<String>,
    },
}
struct Control {
    kind: ControlKind,
    dest: usize,
    target: usize,
    dynamic: usize,
    registry: Option<usize>,
}
impl Control {
    fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        match &mut self.kind {
            ControlKind::Catch(v) | ControlKind::Condition(v) => visit(v),
            ControlKind::Resume { exit, .. } => exit.visit_values(visit),
            ControlKind::Protect { .. } | ControlKind::HandlerBind { .. } => {}
        }
    }
}
/// Unbind N bindings; the first whose restore signals (a watcher, or an
/// integer forwarding given a non-integer by set-default-toplevel-value)
/// is consumed and its escape returned, like unbind_to's pop.
fn checked_unbind(e: &mut Engine<'_>, n: usize) -> Option<Escape> {
    for _ in 0..n {
        if e.vm.unbind_one().is_err() {
            return e.vm.propagated_escape.take();
        }
    }
    None
}

/// GNU signal_or_quit: call the edebug hook before unwinding, borrowing at
/// most twenty levels from the reserve. Use the same suspended-engine/GC
/// discipline as eval/load; no raw Value here crosses a collecting safepoint.
#[cold]
#[inline(never)]
fn signal_hook(e: &mut Engine<'_>, condition: Value, data: Value, failed_depth: Option<usize>) -> Option<Escape> {
    let slot = e.vm.slot("signal-hook-function");
    let hook = e.vm.variable_value(slot).ok()?;
    if hook == Value::NIL || hook == Value::UNBOUND { return None; }
    let r = e.regs;
    let threshold = std::mem::replace(&mut e.vm.gc_threshold, usize::MAX);
    let stack = std::mem::take(&mut e.vm.stack);
    let external = std::mem::take(&mut e.vm.external_roots);
    let saved_depth = e.vm.eval_depth;
    // A refused activation has already incremented GNU's depth, even though
    // no register frame was installed for it.
    let depth = failed_depth.unwrap_or(e.depth_base + e.frames.len());
    e.vm.eval_depth = depth;
    let token = e.vm.max_ensure_room(20);
    e.vm.propagated_escape = None;
    e.vm.propagated_eval_depth = None;
    e.vm.propagated_escape_hooked = false;
    let result = (|| {
        let form = {
            let vm: &Vm = e.vm;
            let quote = |v| Form::List(vec![Form::Sym("quote".into()), Form::Object(handle::FormObject::new(vm, v))]);
            Form::List(vec![Form::Sym("funcall".into()), quote(hook), quote(condition), quote(data)])
        };
        let function = e.vm.compile(&form)?;
        e.vm.propagate_next = true;
        e.vm.execute_raw(function)
    })();
    e.vm.propagate_next = false;
    let replacement = e.vm.propagated_escape.take();
    if let Some(old) = token { e.vm.restore_stack_limits(old); }
    e.vm.eval_depth = saved_depth;
    e.vm.gc_threshold = threshold;
    e.vm.stack = stack;
    e.vm.external_roots = external;
    e.vm.end_nested_execution();
    e.code.resize_with(e.vm.functions.len(), Vec::new);
    // Rebase from the suspended allocation, which may now need a larger
    // width because compiling the hook introduced a wider function.
    if unsafe { widen_registers(e, r, e.vm.register_width) }.is_none() {
        e.error = Some("register storage exhausted".into());
    }
    match (result, replacement) {
        (_, Some(escape)) => Some(escape),
        (Err(error), None) => {
            let message = e.vm.make_lisp_string(error.into_bytes(), true);
            let data = e.vm.list_value(&[message]);
            Some(Escape::Signal(e.vm.intern("error"), data))
        }
        (Ok(_), None) => None,
    }
}
/// Regions belong to register activations. Unwinding consumes those same frames;
/// no record copies a frame pointer, register pointer, or function identity.
fn route_escape(e: &mut Engine<'_>) -> Option<(*mut Value, *const Inst)> {
    let mut original = e.error.take();
    let mut escape = match e.escape.take() {
        Some(escape) => escape,
        None => {
            let text = original.as_deref().unwrap_or("error");
            let condition = if text.contains("unbound variable") {
                "void-variable"
            } else if text.contains("file missing") {
                "file-missing"
            } else if text.contains("file error") {
                "file-error"
            } else if text.contains("invalid function") {
                "invalid-function"
            } else if text.contains("non-function") {
                "void-function"
            } else if text.contains("arity") || text.contains("number of arguments") {
                "wrong-number-of-arguments"
            } else if text.contains("index out of range") {
                "args-out-of-range"
            } else if text.contains("overflow") || text.contains("arithmetic error") {
                "arith-error"
            } else if text.contains("circular") {
                "circular-list"
            } else if text.contains("required") || text.contains("value type") {
                "wrong-type-argument"
            } else {
                "error"
            };
            let slot = e.vm.slot(condition);
            // A plain error keeps its message as data, as GNU's `error' does.
            let data = match original.as_deref() {
                Some(message) if condition == "error" => {
                    let message = e.vm.make_lisp_string(message.as_bytes().to_vec(), true);
                    e.vm.list_value(&[message])
                }
                _ => Value::NIL,
            };
            Escape::Signal(Value::reference(slot + 1, 7), data)
        }
    };
    let failed_depth = e.failed_eval_depth.take();
    e.escape = Some(escape);
    let hooked = std::mem::take(&mut e.escape_hooked);
    if prepare_signal(e, hooked, failed_depth) { original = None; }
    escape = e.escape.take().unwrap();
    if let Escape::Throw(tag, value) = escape
        && (!e.propagate || e.depth_base > 1)
        && !e.vm.signal_handlers.iter()
            .any(|h| matches!(h, SignalHandler::Catch(wanted) if *wanted != Value::NIL && *wanted == tag))
    {
        let slot = e.vm.slot("no-catch");
        let data = e.vm.list_value(&[tag, value]);
        e.escape = Some(Escape::Signal(Value::reference(slot + 1, 7), data));
        prepare_signal(e, false, None);
        escape = e.escape.take().unwrap();
    }
    leave_native(e);
    e.apply_function = None;
    if let Some(mark) = e.apply_mark.take() { e.vm.backtrace.leave(mark); }
    loop {
        while e.controls.len() > e.controls_base {
            let control = e.controls.pop().unwrap();
            // Unwind while each intervening handler is still live: restoring
            // a saved integer forwarding can replace the pending escape.
            if let Some(signal) = checked_unbind(e, e.vm.dynamic.len() - control.dynamic) {
                e.controls.push(control);
                e.escape = Some(signal);
                prepare_signal(e, false, None);
                escape = e.escape.take().unwrap();
                original = None;
                continue;
            }
            let matches = match (&control.kind, &escape) {
                (ControlKind::Protect { .. }, _) => true,
                (ControlKind::Catch(tag), Escape::Throw(wanted, _)) => {
                    *tag != Value::NIL && tag == wanted
                }
                (ControlKind::Condition(_), Escape::Error { target, .. }) => {
                    control.registry.is_some() && control.registry == *target
                }
                _ => false,
            };
            release_control(e, &control);
            if !matches { continue; }
            match control.kind {
                ControlKind::Protect { end } => {
                    e.controls.push(Control {
                        kind: ControlKind::Resume {
                            exit: escape,
                            diagnostic: original,
                        },
                        dest: control.dest,
                        target: end,
                        dynamic: control.dynamic,
                        registry: None,
                    });
                }
                ControlKind::Catch(_) | ControlKind::Condition(_) => {
                    if matches!(control.kind, ControlKind::Condition(_)) {
                        while e.controls.len() > e.controls_base
                            && e.controls.last().is_some_and(|previous| {
                                previous.dest == control.dest
                                    && matches!(previous.kind, ControlKind::Condition(_))
                            })
                        {
                            let sibling = e.controls.pop().unwrap();
                            release_control(e, &sibling);
                        }
                    }
                    let payload = match escape {
                        Escape::Throw(_, value) => value,
                        Escape::Error { object, .. } => object,
                        Escape::Signal(_, _) => unreachable!("signal search normalizes errors"),
                        Escape::Normal(_) => unreachable!(),
                    };
                    // SAFETY: activation-owned compiler/image-validated destination.
                    unsafe {
                        *e.regs.add(control.dest) = payload;
                    }
                }
                ControlKind::Resume { .. } | ControlKind::HandlerBind { .. } => unreachable!(),
            }
            return Some((e.regs, unsafe {
                e.code[e.function].as_ptr().add(control.target)
            }));
        }
        // A normal region completion must never cross a call boundary.
        if matches!(escape, Escape::Normal(_)) {
            e.error = Some("invalid image protect depth".into());
            return None;
        }
        if let Some(signal) = checked_unbind(e, e.vm.dynamic.len() - e.dynamic) {
            e.escape = Some(signal);
            prepare_signal(e, false, None);
            escape = e.escape.take().unwrap();
            original = None;
            continue;
        }
        if let Some(frame) = e.frames.pop() {
            leave_call(e);
            e.rest.leave();
            e.regs = frame.regs;
            e.function = frame.function;
            e.dynamic = frame.dynamic;
            e.controls_base = frame.controls_base;
        } else {
            if let Escape::Error { condition, object, .. } = &escape {
                let (_, data) = e.vm.cons_pair(*object).unwrap();
                let name = e.vm.display(*condition).unwrap_or_default();
                if name == "excessive-lisp-nesting" {
                    original = Some(format!("Lisp nesting exceeds `max-lisp-eval-depth': {}", e.vm.display(data).unwrap_or_default()));
                }
                // GNU's printed error, (CONDITION . DATA), whenever no
                // message was recorded: a condition re-raised across a
                // nested load (`require') keeps its data. void-* always do.
                if matches!(name.as_str(), "void-function" | "void-variable") || original.is_none() {
                    let values = e.vm.display(data).unwrap_or_default();
                    original = Some(format!("({name}{})", values.strip_prefix('(').and_then(|s| s.strip_suffix(')')).filter(|s| !s.is_empty()).map(|s| format!(" {s}")).unwrap_or_default()));
                }
            }
            if e.propagate {
                e.error = Some(original.unwrap_or_else(|| "escape propagated to the eval caller".into()));
                e.vm.propagated_escape = Some(escape);
                e.vm.propagated_escape_hooked = true;
                e.vm.propagated_eval_depth = None;
                return None;
            }
            e.error = Some(original.unwrap_or_else(|| match escape {
                Escape::Throw(_, _) => "no-catch".into(),
                Escape::Signal(condition, _) | Escape::Error { condition, .. } => format!("unhandled signal {}",
                    e.vm.display(condition).unwrap_or_else(|_| "invalid condition".into())),
                Escape::Normal(_) => unreachable!(),
            }));
            return None;
        }
    }
}
// This is argument-stack storage, not Lisp heap objects. Its capacity is reused
// across calls and every live element is forwarded at the engine's safepoints.
#[derive(Default)]
struct RestStack {
    values: Vec<Value>,
    activations: Vec<(usize, Option<OpaqueRest>)>,
}
#[derive(Clone, Copy)]
struct OpaqueRest {
    reg: usize,
    start: usize,
    end: usize,
}
impl RestStack {
    fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for value in &mut self.values {
            visit(value);
        }
    }
    fn leave(&mut self) {
        let (base, _) = self.activations.pop().unwrap();
        self.values.truncate(base);
    }
    fn materialize(&self, vm: &mut Vm, rest: OpaqueRest) -> Value {
        let mut tail = Value::NIL;
        // Arena pushes do not collect; collection is only at explicit strand
        // wait points. No Value copy survives a safepoint in this routine.
        for i in (rest.start..rest.end).rev() {
            vm.conses.push((self.values[i], tail));
            tail = Value::reference(vm.conses.len() - 1, 3);
        }
        tail
    }
}
pub(super) struct Engine<'a> {
    pub(super) vm: &'a mut Vm,
    code: Vec<Vec<Inst>>,
    rest: RestStack,
    stack_used: usize,
    /// The (OFFSET, COUNT) windows `apply` wrote at the end of the
    /// registers, cleared on exit with the used prefix.
    apply_scratch: Vec<(usize, usize)>,
    frames: Vec<Frame>,
    // Physical continuations can own several real calls (an apply chain).
    // K machinery itself has no public activation.
    call_marks: Vec<Option<usize>>,
    native_mark: Option<usize>,
    apply_function: Option<Value>,
    // Until entry, a spreader's calls have no physical callee to unwind them.
    apply_mark: Option<usize>,
    depth_base: usize,
    regs: *mut Value,
    function: usize,
    dynamic: usize,
    error: Option<String>,
    fatal_image_failure: bool,
    pub(super) escape: Option<Escape>,
    escape_hooked: bool,
    failed_eval_depth: Option<usize>,
    /// Lowest native stack address at which an activation may start (R193).
    native_floor: usize,
    /// This engine runs an `eval` form: an escape reaching its outermost frame
    /// is handed to the calling engine instead of becoming an error string.
    propagate: bool,
    // One side stack for all activations. Only [controls_base..] belongs to this one.
    controls: Vec<Control>,
    controls_base: usize,
    budget: i64,
    entries: Vec<EntryCache>,
    inline_bodies: Vec<Option<InlineBody>>,
    free_entries: Vec<usize>,
    // T2 continuation code pointers by index; a register names one by fixnum.
    // Holds no Value, so it is not a root and needs no unwinding.
    continuations: Vec<prim::Then>,
    #[cfg(debug_assertions)]
    resume: Option<(*mut Value, *const Inst)>,
}
impl Engine<'_> {
    fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit_code(&mut self.code, visit);
        for entry in &mut self.entries {
            visit(&mut entry.target);
        }
        self.visit_state_values(visit);
    }
    /// Every Value but the code and entry caches, which `suspend_code` hands
    /// to the collector in place.
    fn visit_state_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        self.rest.visit_values(visit);
        if let Some(function) = &mut self.apply_function { visit(function); }
        for control in &mut self.controls {
            control.visit_values(visit);
        }
        if let Some(exit) = &mut self.escape {
            exit.visit_values(visit);
        }
    }
    /// Root this engine's state across a nested call that may collect: its
    /// code and entries by reference, the rest copied into `external_roots'.
    /// Returns the root mark for `resume_code`.
    /// The code reference is taken last, and from the two fields only: the
    /// caller must reach the Vm through `self.vm` alone until `resume_code`,
    /// which drops the reference before reborrowing the engine.
    pub(super) fn suspend_code(&mut self) -> usize {
        let roots = self.vm.external_roots.len();
        let mut suspended = Vec::new();
        self.visit_state_values(&mut |value| suspended.push(*value));
        self.vm.external_roots.extend(suspended);
        let code = SuspendedCode {
            code: std::ptr::addr_of_mut!(self.code),
            entries: std::ptr::addr_of_mut!(self.entries),
        };
        self.vm.suspended_code.push(code);
        roots
    }
    /// Drop the code reference, then take back the forwarded state values.
    pub(super) fn resume_code(&mut self, roots: usize) {
        self.vm.suspended_code.pop();
        let forwarded = self.vm.external_roots[roots..].to_vec();
        let mut forwarded = forwarded.into_iter();
        self.visit_state_values(&mut |value| *value = forwarded.next().unwrap());
        debug_assert!(forwarded.next().is_none());
        self.vm.external_roots.truncate(roots);
    }
}
#[cfg(test)]
pub(super) fn assert_opaque_rest_compaction() {
    let mut vm = Vm::new();
    // The opaque stack is the ONLY owner of these payloads; stale caller
    // registers must not accidentally make an omitted root traversal pass.
    for _ in 0..20 {
        vm.conses.push((Value::NIL, Value::NIL));
    }
    let a = vm.list_value(&[Value::integer(17).unwrap()]);
    let b = vm.list_value(&[Value::integer(42).unwrap()]);
    let desc = OpaqueRest {
        reg: 0,
        start: 0,
        end: 2,
    };
    let mut e = Engine {
        vm: &mut vm,
        code: vec![],
        rest: RestStack {
            values: vec![a, b],
            activations: vec![(0, Some(desc))],
        },
        stack_used: 0,
        apply_scratch: Vec::new(),
        frames: vec![],
        call_marks: vec![None],
        native_mark: None,
        apply_function: None,
        apply_mark: None,
        depth_base: 0,
        regs: std::ptr::null_mut(),
        function: 0,
        dynamic: 0,
        error: None,
        fatal_image_failure: false,
        escape: None,
        escape_hooked: false,
        failed_eval_depth: None,
        native_floor: 0,
        propagate: false,
        controls: vec![],
        controls_base: 0,
        budget: 0,
        entries: vec![],
        inline_bodies: vec![],
        free_entries: vec![],
        continuations: vec![],
        #[cfg(debug_assertions)]
        resume: None,
    };
    let before = e.vm.heap_stats().allocations;
    assert_eq!(before, 22, "opaque tail must not allocate any conses");
    let mut roots = Vec::new();
    e.visit_values(&mut |v| roots.push(*v));
    assert_eq!(roots.len(), 2, "opaque arguments must be collector roots");
    e.vm.collect(&mut roots);
    let mut roots = roots.into_iter();
    e.visit_values(&mut |v| *v = roots.next().unwrap());
    assert_ne!(e.rest.values[0], a, "fixture must force compaction");
    assert_ne!(e.rest.values[1], b, "fixture must forward every payload");
    assert_eq!(e.vm.heap_stats().reclaimed, 20);
    assert_eq!(e.vm.heap_stats().collections, 1);
    let value = e.rest.materialize(e.vm, desc);
    assert_eq!(
        e.vm.display(value).unwrap(),
        "((17) (42))",
        "compacted opaque arguments must materialize intact"
    );
}

impl Vm {
    pub(crate) fn reader_call(&mut self, function: Value, args: &[Value]) -> Result<Value> {
        let mut stack = std::mem::take(&mut self.stack);
        let roots = self.external_roots.len();
        self.external_roots.extend_from_slice(&stack);
        self.external_roots.push(function);
        self.external_roots.extend_from_slice(args);
        self.propagated_escape = None;
        self.propagated_escape_hooked = false;
        self.propagated_eval_depth = None;
        let result = (|| {
            let quote = |value| {
                Form::List(vec![
                    Form::Sym("quote".into()),
                    Form::Object(crate::handle::FormObject::new(self, value)),
                ])
            };
            let mut call = vec![Form::Sym("funcall".into()), quote(function)];
            call.extend(args.iter().copied().map(quote));
            let (entry, _) = self.compile_request(&Form::List(call))?;
            self.propagate_next = true;
            let result = execute_args(self, entry, &[]);
            self.release_request_entry(entry);
            result
        })();
        self.propagate_next = false;
        // Preserve the allocation: suspended engines retain raw register pointers.
        let stack_end = roots + stack.len();
        stack.copy_from_slice(&self.external_roots[roots..stack_end]);
        self.stack = stack;
        self.external_roots.truncate(roots);
        self.end_nested_execution();
        result
    }

    /// After a nested engine run, whose program the suspended engine does not
    /// own: keep it, registers all NIL again and lowered code intact, for the
    /// next nested run instead of reallocating, refilling and relowering.
    pub(super) fn end_nested_execution(&mut self) {
        let program = std::mem::take(&mut self.execution);
        if !program.registers.is_empty() {
            self.spare_programs.push(program);
        }
    }
    /// Every retained program: the current one and the nested spares.
    pub(super) fn programs_mut(&mut self) -> impl Iterator<Item = &mut Program> {
        std::iter::once(&mut self.execution).chain(self.spare_programs.iter_mut())
    }

    pub(crate) fn pending_eval_escape(&self) -> Option<(Value, Value)> {
        match self.propagated_escape.as_ref() {
            Some(Escape::Signal(condition, data)) => Some((*condition, *data)),
            Some(Escape::Error { condition, object, .. }) => self.cons_pair(*object).ok().map(|(_, data)| (*condition, data)),
            _ => None,
        }
    }
}

pub(super) fn execute(vm: &mut Vm, function: usize) -> Result<Value> {
    execute_args(vm, function, &[])
}
pub(super) fn execute_args(vm: &mut Vm, function: usize, args: &[Value]) -> Result<Value> {
    // Native reentry (eval/load/macro expansion) shares the Lisp budget, but
    // receives another native stack segment before Rust can exhaust it.
    stacker::maybe_grow(256 * 1024, 2 * 1024 * 1024, || {
        let saved = vm.eval_depth;
        vm.eval_depth = saved.saturating_add(1);
        let result = if vm.check_eval_depth().is_err() {
            let condition = vm.intern("excessive-lisp-nesting");
            let data = vm.list_value(&[Value::integer(vm.eval_depth as i64).expect("representable eval depth")]);
            vm.propagated_escape = Some(Escape::Signal(condition, data));
            vm.propagated_escape_hooked = false;
            vm.propagated_eval_depth = Some(vm.eval_depth);
            Err("Lisp nesting exceeds `max-lisp-eval-depth'".into())
        } else {
            execute_args_inner(vm, function, args)
        };
        vm.eval_depth = saved;
        result
    })
}
fn execute_args_inner(vm: &mut Vm, function: usize, args: &[Value]) -> Result<Value> {
    vm.execution_fatal = false;
    vm.require_live_function(function)?;
    if vm.functions[function].lazy.is_some() {
        // No engine is suspended here; nested macro calls get their own and
        // must not consume this execution's propagation request.
        let threshold = std::mem::replace(&mut vm.gc_threshold, usize::MAX);
        let propagate = std::mem::take(&mut vm.propagate_next);
        let forced = vm.force_lazy(function);
        vm.propagate_next = propagate;
        vm.gc_threshold = threshold;
        forced?;
    }
    if let Err(error) = vm.materialize_image_function(function) {
        vm.execution_fatal = true;
        return Err(error);
    }
    let mut program = std::mem::take(&mut vm.execution);
    if program.registers.is_empty() {
        if let Some(spare) = vm.spare_programs.pop() {
            program = spare;
        }
    }
    program.code.resize_with(vm.functions.len(), Vec::new);
    if let Err(error) = lower_function(
        vm,
        &mut program.code,
        &mut program.entries,
        &mut program.inline_bodies,
        &mut program.free_entries,
        function,
    ) {
        vm.execution_fatal = true;
        vm.execution = program;
        return Err(error);
    }
    let size = vm
        .register_width
        .checked_mul(MIN_FRAMES)
        .and_then(|n| n.checked_add(4096))
        .ok_or("register storage overflow")?;
    vm.stack = std::mem::take(&mut program.registers);
    vm.stack.resize(size, Value::NIL);
    vm.stack[..args.len()].copy_from_slice(args);
    let regs = vm.stack.as_mut_ptr();
    let mark = vm.dynamic.len();
    let handler_mark = vm.signal_handlers.len();
    let backtrace_mark = vm.backtrace.frames.len();
    let vm_propagate = std::mem::take(&mut vm.propagate_next);
    program.rest.activations.push((0, None));
    let mut engine = Engine {
        stack_used: vm.functions[function].regs.max(args.len()),
        apply_scratch: Vec::new(),
        depth_base: vm.eval_depth,
        vm,
        code: program.code,
        rest: program.rest,
        frames: Vec::with_capacity(512),
        call_marks: vec![None],
        native_mark: None,
        apply_function: None,
        apply_mark: None,
        regs,
        function,
        dynamic: mark,
        error: None,
        fatal_image_failure: false,
        escape: None,
        escape_hooked: false,
        failed_eval_depth: None,
        native_floor: eval_depth::native_stack_floor(),
        propagate: vm_propagate,
        controls: Vec::new(),
        controls_base: 0,
        budget: 10_000,
        entries: program.entries,
        inline_bodies: program.inline_bodies,
        free_entries: program.free_entries,
        continuations: Vec::new(),
        #[cfg(debug_assertions)]
        resume: None,
    };
    // SAFETY: storage and lowered code satisfy the module invariants above.
    let mut value = unsafe { next(&mut engine, regs, std::ptr::null()) };
    // Unoptimized Rust does not perform sibling-call elimination. Drive the
    // exact same handlers one at a time in dev builds, keeping debug safe.
    loop {
        #[cfg(debug_assertions)]
        while let Some((regs, pc)) = engine.resume.take() {
            value = unsafe { ((*pc).handler)(&mut engine, regs, pc) };
        }
        if !engine.fatal_image_failure
            && (engine.error.is_some() || engine.escape.is_some())
            && let Some((regs, pc)) = route_escape(&mut engine)
        {
            // Region exits are strand wait points too: suspended payloads are
            // activation-owned roots before any cleanup instruction can allocate.
            value = unsafe { poll_resume(&mut engine, regs, pc) };
            continue;
        }
        break;
    }
    // A normal return has already unbound through return_unbind.
    engine.vm.unwind(engine.vm.dynamic.len() - mark);
    engine.vm.signal_handlers.truncate(handler_mark);
    engine.vm.backtrace.leave(backtrace_mark);
    // Only entered frames can have written values. Clear that prefix, not all
    // reserved frames; retaining zeroed storage avoids reinitializing it.
    engine.vm.stack[..engine.stack_used].fill(Value::NIL);
    for &(offset, count) in &engine.apply_scratch {
        engine.vm.stack[offset..offset + count].fill(Value::NIL);
    }
    engine.rest.values.clear();
    engine.rest.activations.clear();
    engine.vm.execution = Program {
        rest: std::mem::take(&mut engine.rest),
        code: std::mem::take(&mut engine.code),
        entries: std::mem::take(&mut engine.entries),
        inline_bodies: std::mem::take(&mut engine.inline_bodies),
        free_entries: std::mem::take(&mut engine.free_entries),
        registers: std::mem::take(&mut engine.vm.stack),
    };
    engine.vm.execution_fatal = engine.fatal_image_failure;
    match engine.error {
        Some(error) => Err(error),
        None => Ok(value),
    }
}
fn lower_function(
    vm: &mut Vm,
    code: &mut Vec<Vec<Inst>>,
    entries: &mut Vec<EntryCache>,
    inline_bodies: &mut Vec<Option<InlineBody>>,
    free_entries: &mut Vec<usize>,
    id: usize,
) -> Result<()> {
    // A variable watcher can compile functions while this engine is
    // suspended at a write, with no resize after it.
    if id >= code.len() { code.resize_with(vm.functions.len().max(id + 1), Vec::new); }
    if code[id].is_empty() {
        vm.materialize_image_function(id)?;
        code[id] = vm.functions[id]
            .code
            .iter()
            .copied()
            .map(|op| {
                let mut inst = lower(vm, op);
                let function = &vm.functions[id];
                if function.rest {
                    let rest_reg = function.captures + function.arity;
                    let mut registers = op;
                    let mut observes = false;
                    registers.map_registers(&mut |reg| {
                        observes |= reg == rest_reg;
                        reg
                    });
                    match op {
                        Op::Call(_, _, start, count) | Op::Direct(_, _, _, start, count) | Op::Prepared(_, _, _, _, start, count) => {
                            observes |= (start..start + count).contains(&rest_reg);
                        }
                        Op::Closure(_, _, site) => {
                            observes |= function.sites[site].contains(&rest_reg);
                        }
                        _ => {}
                    }
                    // Include replacement writes: they discard a pending tail
                    // before a later observation can resurrect its old value.
                    if observes {
                        inst.observed_handler = inst.handler;
                        inst.handler = observe_then;
                    }
                }
                let entry = EntryCache {
                    world_version: 0,
                    target: Value::UNBOUND,
                    function: 0,
                    entry: std::ptr::null(),
                    simple_arity: usize::MAX,
                    primitive: None,
                };
                if let Some(site) = free_entries.pop() {
                    entries[site] = entry;
                    inline_bodies[site] = None;
                    inst.site = site;
                } else {
                    inst.site = entries.len();
                    entries.push(entry);
                    inline_bodies.push(None);
                }
                inst
            })
            .collect();
    }
    Ok(())
}
fn lower(vm: &Vm, op: Op) -> Inst {
    let mut i = Inst {
        handler: cold,
        observed_handler: cold,
        a: 0,
        b: 0,
        c: 0,
        d: 0,
        e: 0,
        value: Value::NIL,
        op,
        site: 0,
        prepared_handler: direct,
    };
    match op {
        Op::Const(a, v) => {
            i.handler = constant;
            i.a = a;
            i.value = v;
        }
        Op::Move(a, b) => {
            i.handler = mov;
            i.a = a;
            i.b = b;
        }
        Op::ResolveHead(a, b) => {
            i.handler = resolve_head;
            i.a = a;
            i.b = b;
        }
        Op::Prepared(a, _, b, c, d, e) => {
            i = lower(vm, Op::Direct(a, b, c, d, e));
            i.prepared_handler = i.handler;
            i.handler = prepared;
            i.op = op;
        }
        Op::Global(a, b) => {
            i.handler = global;
            i.a = a;
            i.b = b;
        }
        Op::ReadCell(a, b) => {
            i.handler = read_cell;
            i.a = a;
            i.b = b;
        }
        Op::WriteCell(a, b) => {
            i.handler = write_cell;
            i.a = a;
            i.b = b;
        }
        Op::Jump(a) => {
            i.handler = jump;
            i.a = a;
        }
        Op::Unless(a, b) => {
            i.handler = unless;
            i.a = a;
            i.b = b;
        }
        Op::Return(a) => {
            i.handler = ret;
            i.a = a;
        }
        Op::Call(a, b, c, d) => {
            i.handler = call;
            i.a = a;
            i.b = b;
            i.c = c;
            i.d = d;
        }
        Op::Direct(a, b, c, d, e) => {
            i.handler = direct;
            i.a = a;
            i.b = b;
            i.c = c;
            i.d = d;
            i.e = e;
            let target = vm.slots[b].function;
            if target.tag() == 6 && prim::generated(target).is_none() {
                i.value = target;
                i.handler = match (prim::legacy(target).expect("legacy primitive"), e) {
                    (Primitive::Car, 1) => traversal::car,
                    (Primitive::Cdr, 1) => traversal::cdr,
                    (Primitive::Null, 1) => traversal::null,
                    (Primitive::Eq, 2) => traversal::eq,
                    (Primitive::Memq, 2) => traversal::memq,
                    (Primitive::Assq, 2) => traversal::assq,
                    (Primitive::PlistGet, 2) => traversal::plist_get,
                    (Primitive::Nth, 2) => traversal::nth,
                    (Primitive::Setcar, 2) => traversal::setcar,
                    (Primitive::Setcdr, 2) => traversal::setcdr,
                    (Primitive::Add, 2) => add,
                    (Primitive::Sub, 2) => sub,
                    (Primitive::Mul, 2) => mul,
                    (Primitive::Inc, 1) => inc,
                    (Primitive::Dec, 1) => dec,
                    (Primitive::Less, 2) => less,
                    (Primitive::Equal, 2) => equal,
                    (Primitive::Cons, 2) => cons,
                    (Primitive::Remainder, 2) => remainder,
                    (Primitive::Aref, 2) | (Primitive::Length, 1) => sequence_read,
                    (Primitive::CharAfter, 0 | 1)
                    | (
                        Primitive::Point
                        | Primitive::PointMin
                        | Primitive::PointMax
                        | Primitive::BufferSize,
                        0,
                    )
                    | (Primitive::GotoChar, 1) => buffer_read,
                    _ => direct,
                };
            } else if e == 1 && prim::generated(target).is_some_and(|d| d.name == "insert") {
                i.value = target;
                i.handler = insert_string;
            }
        }
        _ => {}
    }
    i
}
// Keep observation scratch out of dispatch handlers. Inlining this register
// walk into next gives handlers address-taken locals that prevent sibling calls
// in optimized builds, retaining native frames even in non-rest programs.
#[inline(never)]
unsafe fn observe_rest(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) {
    // A pending binding cannot be copied into an ordinary Value. Materialize
    // before any instruction that can read it, including implicit captures and
    // contiguous call windows. Conservative destination matches are intentional.
    if let Some(rest) = e.rest.activations.last().and_then(|(_, rest)| *rest) {
        let mut op = unsafe { (*pc).op };
        // These writes cannot fail or transfer control before replacing the
        // binding. Its previous tail is genuinely dead, not observed.
        if matches!(op, Op::Const(dst, _) if dst == rest.reg)
            || matches!(op, Op::Move(dst, src) if dst == rest.reg && src != rest.reg)
        {
            e.rest.activations.last_mut().unwrap().1 = None;
            return;
        }
        let mut observed = false;
        op.map_registers(&mut |reg| {
            observed |= reg == rest.reg;
            reg
        });
        match op {
            Op::Call(_, _, start, count) | Op::Direct(_, _, _, start, count) | Op::Prepared(_, _, _, _, start, count) => {
                observed |= (start..start + count).contains(&rest.reg);
            }
            Op::Closure(_, _, site) => {
                observed |= e.vm.functions[e.function].sites[site].contains(&rest.reg);
            }
            _ => {}
        }
        if observed {
            let value = e.rest.materialize(e.vm, rest);
            unsafe {
                *r.add(rest.reg) = value;
            }
            e.rest.activations.last_mut().unwrap().1 = None;
        }
    }
}
// Observation may allocate, but cannot collect or reenter dispatch. The rest
// side stack and register window remain traced by Engine::visit_values.
unsafe fn observe_then(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        observe_rest(e, r, pc);
        ((*pc).observed_handler)(e, r, pc)
    }
}
#[inline(always)]
unsafe fn next(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    // A null pc is used only at entry, never stored in an instruction/frame.
    let pc = if pc.is_null() {
        e.code[e.function].as_ptr()
    } else {
        pc
    };
    #[cfg(debug_assertions)]
    {
        e.resume = Some((r, pc));
        Value::UNBOUND
    }
    #[cfg(not(debug_assertions))]
    unsafe {
        // No per-instruction stack probe: it tripled straight-line dispatch
        // cost. Native stack grows per Lisp activation (T2 callbacks, cold
        // non-tail handlers), so `depth_room` checks remaining bytes there.
        ((*pc).handler)(e, r, pc)
    }
}
#[inline(always)]
unsafe fn advance(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe { next(e, r, pc.add(1)) }
}
#[cold]
#[inline(never)]
fn native_stack_failure(e: &mut Engine<'_>, message: &str) -> Value {
    let value = e.vm.make_lisp_string(message.as_bytes().to_vec(), true);
    e.error = Some(message.to_owned());
    raise(e, "error", &[value])
}
#[cold]
#[inline(always)]
fn raise(e: &mut Engine<'_>, condition: &str, values: &[Value]) -> Value {
    let slot = e.vm.slot(condition);
    let data = e.vm.list_value(values);
    e.escape = Some(Escape::Signal(Value::reference(slot + 1, 7), data));
    Value::UNBOUND
}
#[cold]
#[inline(always)]
fn wrong_type(e: &mut Engine<'_>, predicate: &str, value: Value) -> Value {
    data::type_error(e, predicate, value)
}
/// GNU bytecode.c:538-541: a compiled function called with the wrong count
/// signals `(wrong-number-of-arguments (MANDATORY . NONREST) COUNT)`; NONREST
/// counts required and optional parameters even when there is a &rest.
/// Allocates only after every argument is read.
#[cold]
#[inline(never)]
fn closure_arity_error(e: &mut Engine<'_>, function: usize, argc: usize) -> Value {
    let f = &e.vm.functions[function];
    let min = Value::integer(f.required as i64).unwrap();
    let max = Value::integer(f.arity as i64).unwrap();
    e.vm.conses.push((min, max));
    let arity = Value::reference(e.vm.conses.len() - 1, 3);
    raise(e, "wrong-number-of-arguments", &[arity, Value::integer(argc as i64).unwrap()])
}
/// GNU eval.c funcall_general: a function cell that resolves to nothing is
/// `void-function'; any other non-function object (a string, a number, a
/// vector, a cons that is not a lambda list) is `invalid-function'.
#[cold]
fn uncallable_condition(_e: &mut Engine<'_>, target: Value) -> &'static str {
    if target == Value::UNBOUND || target == Value::NIL || target == Value::TRUE {
        return "void-function";
    }
    "invalid-function"
}
#[cold]
fn fail(e: &mut Engine<'_>, message: &str) -> Value {
    e.error = Some(message.into());
    std::hint::black_box(Value::UNBOUND)
}
#[cold]
#[inline(never)]
unsafe fn poll_resume(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    e.budget = 10_000;
    // R193: the dispatch below must be a sibling call or every poll keeps a
    // frame: gdb on the gate-built (non-LTO) romacs-gnu-ert adapter showed
    // 74,600 poll_resume frames and SIGABRT under `(while t)` when the
    // collection's Vec lived in this frame. Collection is therefore out of
    // line, and the engine's native floor still refuses recoverably should a
    // build keep the frame anyway. Cold path only.
    if eval_depth::stack_pointer() < e.native_floor {
        return native_stack_failure(e, "native stack exhausted during Lisp evaluation");
    }
    if e.vm.heap_objects() >= e.vm.gc_threshold {
        poll_collect(e);
    }
    unsafe { next(e, r, pc) }
}
#[cold]
#[inline(never)]
fn poll_collect(e: &mut Engine<'_>) {
    // Forward in place without moving instruction buffers: suspended PCs and
    // entry pointers stay valid.
    let roots = e.suspend_code();
    e.vm.collect(&mut []);
    e.resume_code(roots);
}
// Like GNU bytecode.c:638-646, only a plain, bound symbol takes the slot
// read. Inspect the live set-only redirect on every entry: localization need
// not publish a function-world epoch, and can happen inside a cached callee.
// No Value or slot pointer is retained across a safepoint or allocation.
unsafe fn global(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let i = &*pc;
        if i.b == e.vm.multibyte_slot
            || i.b == e.vm.truncate_lines_slot
            || e.vm.variable_redirects.get(i.b) != Some(&Some(VariableRedirect::Plain))
        {
            return cold(e, r, pc);
        }
        let value = e.vm.slots[i.b].value;
        if value == Value::UNBOUND {
            return cold(e, r, pc);
        }
        *r.add(i.a) = value;
        advance(e, r, pc)
    }
}
unsafe fn constant(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        *r.add((*pc).a) = (*pc).value;
        advance(e, r, pc)
    }
}
#[cfg_attr(
    all(target_os = "linux", target_arch = "x86_64"),
    unsafe(link_section = ".text.vm2.mov")
)]
unsafe fn mov(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        *r.add((*pc).a) = *r.add((*pc).b);
        advance(e, r, pc)
    }
}
unsafe fn read_cell(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let v = *r.add((*pc).b);
        if v.tag() != 5 {
            return fail(e, "cell required");
        }
        *r.add((*pc).a) = e.vm.read_capture_cell(v.payload() as usize);
        advance(e, r, pc)
    }
}
unsafe fn write_cell(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let v = *r.add((*pc).a);
        if v.tag() != 5 {
            return fail(e, "cell required");
        }
        e.vm.write_capture_cell(v.payload() as usize, *r.add((*pc).b));
        advance(e, r, pc)
    }
}
#[cfg_attr(
    all(target_os = "linux", target_arch = "x86_64"),
    unsafe(link_section = ".text.vm2.jump")
)]
unsafe fn jump(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let to = e.code.get_unchecked(e.function).as_ptr().add((*pc).a);
        if to <= pc {
            e.budget -= 1;
            if e.budget < 0 {
                return poll_resume(e, r, to);
            }
        }
        next(e, r, to)
    }
}
#[cfg_attr(
    all(target_os = "linux", target_arch = "x86_64"),
    unsafe(link_section = ".text.vm2.unless")
)]
unsafe fn unless(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        if *r.add((*pc).a) == Value::NIL {
            next(e, r, e.code.get_unchecked(e.function).as_ptr().add((*pc).b))
        } else {
            advance(e, r, pc)
        }
    }
}
unsafe fn ret(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let value = *r.add((*pc).a);
        if e.vm.dynamic.len() != e.dynamic {
            return return_unbind(e, r, pc);
        }
        if e.controls.len() != e.controls_base {
            return fail(e, "invalid image return through control region");
        }
        if let Some(frame) = e.frames.pop() {
            leave_call(e);
            e.rest.leave();
            *frame.regs.add(frame.dest) = value;
            e.function = frame.function;
            e.dynamic = frame.dynamic;
            e.controls_base = frame.controls_base;
            e.regs = frame.regs;
            next(e, frame.regs, frame.pc)
        } else {
            value
        }
    }
}
#[cold]
#[inline(never)]
unsafe fn return_unbind(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    if let Some(signal) = checked_unbind(e, e.vm.dynamic.len() - e.dynamic) {
        e.escape = Some(signal);
        return Value::UNBOUND;
    }
    unsafe { ret(e, r, pc) }
}
// Resolution is reach-time, not compile-time. Autoload loading can rebase the
// register stack; never retain the pre-load pointer or target across that call.
unsafe fn resolve_head(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let function = Value::reference((*pc).b + 1, 7);
        let Some(target) = t2_resolve(e, function) else { return Value::UNBOUND; };
        if target.tag() == 3 && e.vm.autoload_object(target) {
            return match autoload_then(e, r, function, target) {
                Some(r) => resolve_head(e, r, pc),
                None => Value::UNBOUND,
            };
        }
        if !matches!(target.tag(), 4 | 6) && !lambda_list(e, target) {
            let condition = uncallable_condition(e, target);
            return raise(e, condition, &[function]);
        }
        *r.add((*pc).a) = target;
        advance(e, r, pc)
    }
}
unsafe fn prepared(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let Op::Prepared(_, snapshot, _, _, _, _) = (*pc).op else { unreachable!() };
        let target = *r.add(snapshot);
        // Keep direct-call speculation when the snapshot still names the cell.
        // A changed cell must NOT replace the function already resolved.
        if target == e.vm.slots[(*pc).b].function {
            ((*pc).prepared_handler)(e, r, pc)
        } else {
            invoke(e, r, pc, target, (*pc).d, (*pc).e)
        }
    }
}
/// Check the prospective activation, independently of register storage.
/// The common case (limit >= 100, depth and native stack within bounds) is
/// two compares; everything else, including GNU's clamp, takes the cold path.
#[inline(always)]
fn depth_room(e: &mut Engine<'_>) -> bool {
    let attempted = e.depth_base + e.frames.len() + 1;
    let limit = e.vm.forwarded.eval_limit();
    if limit >= 100 && attempted as u64 <= limit as u64 && eval_depth::stack_pointer() >= e.native_floor {
        return true;
    }
    depth_room_cold(e, attempted)
}
#[cold]
#[inline(never)]
fn depth_room_cold(e: &mut Engine<'_>, attempted: usize) -> bool {
    // R193: refuse on remaining native BYTES, independently of the Lisp
    // counter, while there is still room to build and unwind a Lisp error.
    if eval_depth::stack_pointer() < e.native_floor {
        native_stack_failure(e, "native stack exhausted during Lisp evaluation");
        return false;
    }
    let saved = e.vm.eval_depth;
    e.vm.eval_depth = attempted;
    let fits = e.vm.check_eval_depth().is_ok();
    e.vm.eval_depth = saved;
    if !fits {
        e.failed_eval_depth = Some(attempted);
        raise(e, "excessive-lisp-nesting", &[Value::integer(attempted as i64).expect("depth")]);
    }
    fits
}
/// Activations the register storage holds before it first widens. Each is
/// reserved at the widest function compiled so far, so a single wide
/// function multiplies this; deeper calls widen on demand (`frame_room`).
const MIN_FRAMES: usize = 17;
/// Grow only on a cold storage boundary, rebasing all activation pointers.
unsafe fn frame_room(e: &mut Engine<'_>, r: *mut Value) -> Option<*mut Value> {
    let needed = (e.frames.len() + 3).checked_mul(e.vm.register_width)?;
    if needed <= e.vm.stack.len().checked_sub(4096)? {
        Some(r)
    } else {
        unsafe { widen_registers(e, r, e.vm.register_width) }
    }
}
// Keep metadata allocation and cleanup out of threaded callers so optimized
// non-LTO builds can tail-dispatch after the operation completes.
#[inline(never)]
fn leave_native(e: &mut Engine<'_>) {
    if let Some(mark) = e.native_mark.take() { e.vm.backtrace.leave(mark); }
}
#[inline(never)]
fn leave_call(e: &mut Engine<'_>) {
    if let Some(mark) = e.call_marks.pop().flatten() { e.vm.backtrace.leave(mark); }
}
#[inline(never)]
unsafe fn trace_call(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst,
    target: Value, start: usize, argc: usize) -> usize {
    unsafe {
        let function = if std::ptr::eq(pc, &K_CODE[0]) { *r.add(K_F) } else {
            match (*pc).op {
                Op::Direct(_, slot, ..) | Op::Prepared(_, _, slot, ..) => Value::reference(slot + 1, 7),
                Op::Call(_, function, ..) => *r.add(function),
                _ => target,
            }
        };
        // The spreader carries the final original function through nested
        // apply calls. Reading the outer call's first argument loses aliases
        // and can mislabel a real failing callee as apply itself.
        let function = e.apply_function.take().unwrap_or(function);
        let resolved = if target == Value::UNBOUND { e.vm.backtrace_resolve(function).unwrap_or(target) } else { target };
        e.vm.backtrace.enter(function, resolved, std::slice::from_raw_parts(r.add(start), argc), e.vm.dynamic.len())
    }
}
#[inline(never)]
unsafe fn enter_native(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst,
    target: Value, start: usize, argc: usize) {
    let mark = if let Some(mark) = e.native_mark.take() {
        e.vm.backtrace.resolve_pending(target);
        mark
    } else {
        unsafe { trace_call(e, r, pc, target, start, argc) }
    };
    e.native_mark = Some(e.apply_mark.take().unwrap_or(mark));
}
#[inline(never)]
unsafe fn enter_apply(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst, start: usize, argc: usize) {
    let target = Value::reference(Primitive::Apply as usize, 6);
    let mark = if let Some(mark) = e.native_mark.take() {
        e.vm.backtrace.resolve_pending(target);
        mark
    } else {
        unsafe { trace_call(e, r, pc, target, start, argc) }
    };
    // A callee's physical activation owns its whole native apply chain. This
    // keeps genuine apply operations visible until the final callee leaves.
    e.apply_mark.get_or_insert(mark);
}
unsafe fn call(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe { invoke(e, r, pc, *r.add((*pc).b), (*pc).c, (*pc).d) }
}
unsafe fn direct(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let i = &*pc;
        let entry = *e.entries.get_unchecked(i.site);
        // A call-site safe point: every argument and the caller continuation
        // still have the generic layout. Check at EVERY speculative body entry;
        // no purity analysis discharges a check across arbitrary Lisp calls.
        // Nothing may allocate, collect, or invoke Lisp between here and entry.
        if entry.world_version != 0
            && entry.world_version == e.vm.world_version
            && entry.simple_arity == i.e
        {
            #[cfg(test)]
            {
                e.vm.speculative_hits += 1;
            }
            if !depth_room(e) { return Value::UNBOUND; }
            let Some(r) = frame_room(e, r) else { return fail(e, "register storage overflow"); };
            if let Some(body) = e.inline_bodies.get_unchecked(i.site)
                && let Some(value) = body.evaluate(r.add(i.d))
            {
                // Poll with the original argument layout, never a copied Value
                // across collection. Resume rechecks the epoch at this same pc.
                e.budget -= 1;
                if e.budget < 0 {
                    return poll_resume(e, r, pc);
                }
                #[cfg(test)]
                {
                    e.vm.inline_hits += 1;
                }
                *r.add(i.a) = value;
                return advance(e, r, pc);
            }
            let new = r.add(e.vm.functions.get_unchecked(e.function).regs.max(1));
            std::ptr::copy(r.add(i.d), new, i.e);
            return enter_frame(e, r, pc, new, entry.function, None, entry.target, i.d, i.e);
        }
        #[cfg(test)]
        if entry.world_version != 0 && entry.world_version != e.vm.world_version {
            e.vm.speculative_misses += 1;
        }
        // Jump-only deopt: no registers, frames, or control regions changed.
        direct_generic(e, r, pc)
    }
}
#[cold]
unsafe fn direct_generic(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let i = &*pc;
        // Function-cell equality is an authoritative guard, including redefinition
        // back to the original primitive; never assume a sealed symbol table.
        let slot = e.vm.slots.get_unchecked(i.b);
        let cache = e.vm.caches.get_unchecked_mut(i.c);
        if cache.version != slot.version {
            if e.vm.macros.contains(&i.b) && !e.vm.expanding.contains(&i.b) {
                return fail(
                    e,
                    "invalid function: macro dependency changed; resubmit source",
                );
            }
            cache.version = slot.version;
            cache.target = slot.function;
        }
        let target = cache.target;
        if target == Value::UNBOUND || target == Value::NIL {
            return raise(e, "void-function", &[Value::reference(i.b + 1, 7)]);
        }
        // An autoload cell is resolved from its symbol, which it must name.
        if target.tag() == 7 || target.tag() == 3 && (e.vm.autoload_object(target) || lambda_list(e, target)) {
            return invoke_full(e, r, pc, Value::reference(i.b + 1, 7), i.d, i.e);
        }
        // GNU eval.c:3158-3160 names the called SYMBOL, not its cell's value.
        if !matches!(target.tag(), 4 | 6) {
            let condition = uncallable_condition(e, target);
            return raise(e, condition, &[Value::reference(i.b + 1, 7)]);
        }
        // Refresh an invalidated speculation lazily; ineligible closures retain
        // the existing function-id cache instead of resolving on every call.
        if target.tag() == 4 && e.entries[i.site].world_version != 0 {
            return resolve_entry(e, r, pc, target, i.d, i.e);
        }
        invoke(e, r, pc, target, i.d, i.e)
    }
}
#[inline(always)]
unsafe fn invoke(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        if target.tag() != 4 {
            let entry = *e.entries.get_unchecked((*pc).site);
            if entry.target == target && entry.primitive.is_some() {
                return cached_primitive(e, r, pc, target, start, argc);
            }
            return resolve_entry(e, r, pc, target, start, argc);
        }
        let Some(closure) = e.vm.closures.get(target.payload() as usize) else {
            return fail(e, "invalid closure reference");
        };
        let function = closure.function;
        let entry = *e.entries.get_unchecked((*pc).site);
        if entry.function != function || entry.entry.is_null() {
            return resolve_entry(e, r, pc, target, start, argc);
        }
        if !depth_room(e) { return Value::UNBOUND; }
        let Some(r) = frame_room(e, r) else { return fail(e, "register storage overflow"); };
        if entry.simple_arity == argc {
            let new = r.add(e.vm.functions.get_unchecked(e.function).regs.max(1));
            let mark = call_mark(e, r, pc, target, start, argc);
            match argc {
                0 => {}
                1 => *new = *r.add(start),
                2 => {
                    *new = *r.add(start);
                    *new.add(1) = *r.add(start + 1);
                }
                n => std::ptr::copy(r.add(start), new, n),
            }
            return enter_frame_marked(e, r, pc, new, function, None, mark);
        }
        let closure = &e.vm.closures[target.payload() as usize];
        let f = e.vm.functions.get_unchecked(function);
        if argc < f.required || (!f.rest && argc > f.arity) {
            enter_native(e, r, pc, target, start, argc);
            return closure_arity_error(e, function, argc);
        }
        let new = r.add(e.vm.functions.get_unchecked(e.function).regs.max(1));
        let dst = new.add(f.captures);
        let positional = argc.min(f.arity);
        // The arguments may lie where the new frame goes (`apply`'s spread
        // window once storage has widened): read them all before writing
        // the frame (the backtrace entry, then rest values, then the
        // positional ones by memmove) and only then write the captures.
        let mark = call_mark(e, r, pc, target, start, argc);
        let f = e.vm.functions.get_unchecked(function);
        let closure = &e.vm.closures[target.payload() as usize];
        let rest = if f.rest {
            let begin = e.rest.values.len();
            for i in positional..argc {
                e.rest.values.push(*r.add(start + i));
            }
            Some(OpaqueRest {
                reg: f.captures + f.arity,
                start: begin,
                end: e.rest.values.len(),
            })
        } else {
            None
        };
        std::ptr::copy(r.add(start), dst, positional);
        match closure.captures.len() {
            0 => {}
            1 => *new = closure.captures[0],
            2 => {
                *new = closure.captures[0];
                *new.add(1) = closure.captures[1];
            }
            n => std::ptr::copy_nonoverlapping(closure.captures.as_ptr(), new, n),
        }
        for i in positional..f.arity {
            *dst.add(i) = Value::NIL;
        }
        if f.rest {
            *dst.add(f.arity) = Value::NIL;
        }
        enter_frame_marked(e, r, pc, new, function, rest, mark)
    }
}
#[cold]
#[inline(never)]
unsafe fn resolve_entry(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        let mut r = r;
        if target.tag() == 4 {
            let Some(closure) = e.vm.closures.get(target.payload() as usize) else {
                return fail(e, "invalid closure reference");
            };
            let function = closure.function;
            if e.vm.functions[function].lazy.is_some() {
                // Collection stays inhibited throughout, so `target` and the
                // argument registers keep their indices; only `r` is rebased.
                match force_lazy(e, r, function) {
                    Some(rebased) => r = rebased,
                    None => return Value::UNBOUND,
                }
            }
            let closure = &e.vm.closures[target.payload() as usize];
            let world_version = if matches!((*pc).op, Op::Direct(..) | Op::Prepared(..))
                && closure.captures.is_empty()
                && e.vm.functions[function].required == e.vm.functions[function].arity
                && !e.vm.functions[function].rest
                && e.vm.slots[(*pc).b].function == target
                && !e.vm.macros.contains(&(*pc).b)
            {
                e.vm.world_version
            } else {
                0
            };
            if let Err(error) = lower_function(
                e.vm,
                &mut e.code,
                &mut e.entries,
                &mut e.inline_bodies,
                &mut e.free_entries,
                function,
            ) {
                e.fatal_image_failure = true;
                e.error = Some(error);
                return Value::UNBOUND;
            }
            e.inline_bodies[(*pc).site] = if world_version != 0 {
                #[cfg(test)]
                if e.vm.disable_inline {
                    None
                } else {
                    inline_body(e.vm, function)
                }
                #[cfg(not(test))]
                inline_body(e.vm, function)
            } else {
                None
            };
            *e.entries.get_unchecked_mut((*pc).site) = EntryCache {
                world_version,
                // A function id is stable through moving GC. Never retain one
                // particular environment as the cache key or copy its captures.
                target: Value::UNBOUND,
                function,
                entry: e.code.get_unchecked(function).as_ptr(),
                primitive: None,
                simple_arity: {
                    let f = &e.vm.functions[function];
                    if f.captures == 0 && !f.rest && f.required == f.arity {
                        f.arity
                    } else {
                        usize::MAX
                    }
                },
            };
            return invoke(e, r, pc, target, start, argc);
        }
        if prim::generated(target).is_some() {
            return enter_generated(e, r, pc, target, start, argc);
        }
        if target.tag() == 6 {
            let Some(p) = prim::legacy(target) else {
                return fail(e, "invalid primitive reference");
            };
            // These own control transfer or stack rebasing, not ordinary values.
            if !matches!(
                p,
                Primitive::Apply
                    | Primitive::Throw
                    | Primitive::Signal
                    | Primitive::Load
                    | Primitive::Require
                    | Primitive::Eval
                    | Primitive::Macroexpand
                    | Primitive::AutoloadDoLoad
                    | Primitive::ReadHostEvent
                    | Primitive::FormatModeLine
                    | Primitive::MakeInterpretedClosure
            ) {
                let call = primitive_target(p);
                e.inline_bodies[(*pc).site] = None;
                *e.entries.get_unchecked_mut((*pc).site) = EntryCache {
                    world_version: 0,
                    target,
                    function: usize::MAX,
                    entry: std::ptr::null(),
                    simple_arity: usize::MAX,
                    primitive: Some(call),
                };
                return cached_primitive(e, r, pc, target, start, argc);
            }
        }
        invoke_full(e, r, pc, target, start, argc)
    }
}
#[inline(always)]
unsafe fn invoke_full(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        let function = target;
        // Out of line: the alias walk's address-taken scratch would stop
        // every tail dispatch below, so a funcall or apply in a loop grew
        // the native stack.
        let Some(target) = t2_resolve(e, target) else {
            return Value::UNBOUND;
        };
        if target == Value::reference(Primitive::Apply as usize, 6) {
            return apply(e, r, pc, start, argc);
        }
        if target == Value::reference(Primitive::Throw as usize, 6)
            || target == Value::reference(Primitive::Signal as usize, 6)
        {
            enter_native(e, r, pc, target, start, argc);
            let signal = target == Value::reference(Primitive::Signal as usize, 6);
            if !(argc == 2 || (signal && argc == 1)) {
                return fail(e, "wrong primitive arity");
            }
            if signal {
                // eval.c Fsignal/signal_or_quit: an error object with DATA
                // omitted re-signals itself; nil with non-list DATA is `error'.
                let data = if argc == 2 { *r.add(start + 1) } else { Value::NIL };
                let mut condition = *r.add(start);
                if condition.tag() == 3 && data == Value::NIL {
                    let Ok((condition, data)) = e.vm.cons_pair(condition) else { return fail(e, "invalid error object") };
                    e.escape = Some(Escape::Signal(condition, data));
                    return Value::UNBOUND;
                }
                if condition == Value::NIL && data.tag() != 3 {
                    condition = e.vm.intern("error");
                }
                e.escape = Some(Escape::Signal(condition, data));
                if condition != Value::NIL
                    && condition != Value::TRUE
                    && (condition.tag() != 7 || condition == Value::UNBOUND)
                {
                    e.escape = None;
                    return wrong_type(e, "symbolp", condition);
                }
                return Value::UNBOUND;
            }
            e.escape = Some(Escape::Throw(*r.add(start), *r.add(start + 1)));
            return Value::UNBOUND;
        }
        if target.tag() == 6 {
            if target == Value::reference(Primitive::Load as usize, 6)
                || target == Value::reference(Primitive::Require as usize, 6)
                || target == Value::reference(Primitive::Eval as usize, 6)
                || target == Value::reference(Primitive::MakeInterpretedClosure as usize, 6)
                || target == Value::reference(Primitive::Macroexpand as usize, 6)
                || target == Value::reference(Primitive::AutoloadDoLoad as usize, 6)
                || target == Value::reference(Primitive::ReadHostEvent as usize, 6)
                || target == Value::reference(Primitive::FormatModeLine as usize, 6)
            {
                return primitive(e, r, pc, target, start, argc);
            }
            return invoke(e, r, pc, target, start, argc);
        }
        // eval.c:2760/3176/3530: an autoload loads its file, then the call is
        // retried from the original function (`goto retry').
        if target.tag() == 3 && e.vm.autoload_object(target) {
            enter_native(e, r, pc, target, start, argc);
            return match autoload_then(e, r, function, target) {
                Some(r) => t2_invoke_full(e, r, pc, function, start, argc),
                None => Value::UNBOUND,
            };
        }
        if lambda_list(e, target) {
            enter_native(e, r, pc, target, start, argc);
            if let Some(failure) = lambda_list_failure(e.vm, target, std::slice::from_raw_parts(r.add(start), argc).to_vec()) {
                return lambda_list_raise(e, failure);
            }
            return match lambda_list_then(e, r, target) {
                Some((r, closure)) => t2_invoke_full(e, r, pc, closure, start, argc),
                None => Value::UNBOUND,
            };
        }
        if target.tag() != 4 {
            return {
                let condition = uncallable_condition(e, target);
                raise(e, condition, &[function])
            };
        }
        invoke(e, r, pc, target, start, argc)
    }
}
/// Cold apply adapter. Its bounded scratch window is disjoint from all frames.
/// Arguments are copied into the callee before the next handler executes; nested
/// calls therefore never retain a pointer into the shared scratch window.
#[cold]
unsafe fn apply(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        enter_apply(e, r, pc, start, argc);
        let base = r.offset_from(e.vm.stack.as_ptr()) as usize;
        // Out of line: the Result's error drop would stop the tail dispatch.
        let Some((target, offset, count)) = t2_prepare_apply(e, base + start, argc) else {
            return Value::UNBOUND;
        };
        e.apply_function = Some(target);
        invoke_full(e, r, pc, target, offset - base, count)
    }
}
#[inline(always)]
unsafe fn enter_frame(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    new: *mut Value,
    function: usize,
    rest: Option<OpaqueRest>,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        let mark = call_mark(e, r, pc, target, start, argc);
        enter_frame_marked(e, r, pc, new, function, rest, mark)
    }
}
/// The backtrace entry of the call about to enter: its arguments are read
/// here, so a caller whose arguments may lie under the new frame (`apply`'s
/// spread window) takes it before writing the frame.
#[inline(always)]
unsafe fn call_mark(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst, target: Value, start: usize, argc: usize) -> usize {
    unsafe {
        if let Some(mark) = e.native_mark.take() {
            e.vm.backtrace.resolve_pending(target);
            mark
        } else {
            trace_call(e, r, pc, target, start, argc)
        }
    }
}
#[inline(always)]
unsafe fn enter_frame_marked(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    new: *mut Value,
    function: usize,
    rest: Option<OpaqueRest>,
    mark: usize,
) -> Value {
    unsafe {
        e.call_marks.push(Some(e.apply_mark.take().unwrap_or(mark)));
        let base = rest.map_or(e.rest.values.len(), |rest| rest.start);
        e.rest.activations.push((base, rest));
        // Captures/arguments have already been copied, including on lower failure.
        let end = new.offset_from(e.vm.stack.as_ptr()) as usize + e.vm.functions[function].regs;
        e.stack_used = e.stack_used.max(end);
        // The function-id entry cache guarantees lowering before frame entry.
        e.frames.push(Frame {
            pc: pc.add(1),
            regs: r,
            dest: (*pc).a,
            function: e.function,
            dynamic: e.dynamic,
            controls_base: e.controls_base,
        });
        e.controls_base = e.controls.len();
        e.regs = new;
        e.function = function;
        e.dynamic = e.vm.dynamic.len();
        let pc = e.code.get_unchecked(function).as_ptr();
        e.budget -= 1;
        if e.budget < 0 {
            return poll_resume(e, new, pc);
        }
        next(e, new, pc)
    }
}
// T2 call requests (PRIMITIVE-DOCTRINE section 3). A T2 primitive returns
// `Step::Call` instead of calling Lisp. The engine then pushes a CONTINUATION
// activation: an ordinary Frame plus rest activation whose registers begin where
// a callee's would, laid out as
//   [K_RES, K_THEN, K_F, K_SAVED_N, K_ARGS_N, saved.., args..].
// It runs the three fixed instructions of K_CODE: call F on the arguments
// (`t2_call`), hand the value and the saved registers to the continuation
// (`t2_resume`), and return K_RES to the primitive's caller (the ordinary
// `ret`). Saved Values are therefore registers of a live frame: `Vm::collect`
// traces and forwards the whole register stack, and an escape pops the
// activation like any other frame. The continuation is a `fn` pointer, named by
// a fixnum register through `Engine::continuations`, so it captures nothing.
//
// While the activation runs, `e.function` stays the primitive caller's. No
// K_CODE instruction reads it; a callee's base is the activation's own width.
const K_RES: usize = 0;
const K_THEN: usize = 1;
const K_F: usize = 2;
const K_SAVED_N: usize = 3;
const K_ARGS_N: usize = 4;
const K_SAVED: usize = 5;
const fn k_inst(handler: Handler) -> Inst {
    Inst {
        handler,
        observed_handler: handler,
        a: K_RES,
        b: 0,
        c: 0,
        d: 0,
        e: 0,
        value: Value::NIL,
        op: Op::Return(K_RES),
        // Never an entry-cache index: K_CODE calls do not use the site caches.
        site: usize::MAX,
        prepared_handler: direct,
    }
}
static K_CODE: [Inst; 3] = [k_inst(t2_call), k_inst(t2_resume), k_inst(ret)];
fn k_count(v: Value) -> usize {
    v.integer_value().expect("continuation count register") as usize
}
/// Registers used by the continuation activation at `k`.
unsafe fn k_width(k: *mut Value) -> usize {
    unsafe { K_SAVED + k_count(*k.add(K_SAVED_N)) + k_count(*k.add(K_ARGS_N)) }
}
/// Where a handler continues after a Step. Only scalars cross back into the
/// handler, so it keeps its sibling-call dispatch: the Step's argument vectors
/// are owned and dropped inside the non-inlined helpers below, never live
/// across `next` (which would retain a native frame per callback).
enum Continue {
    At(*mut Value, *const Inst),
    Poll(*mut Value, *const Inst),
    Stop,
}
#[inline(always)]
unsafe fn continue_at(e: &mut Engine<'_>, to: Continue) -> Value {
    unsafe {
        match to {
            Continue::At(r, pc) => next(e, r, pc),
            Continue::Poll(r, pc) => poll_resume(e, r, pc),
            Continue::Stop => Value::UNBOUND,
        }
    }
}
/// Enter a generated primitive from a call site (a caller's instruction or
/// `K_CODE[0]`, when the callback is itself a generated primitive).
#[cold]
#[inline(never)]
unsafe fn enter_generated(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        let to = generated_step(e, r, pc, target, start, argc);
        continue_at(e, to)
    }
}
#[inline(never)]
unsafe fn generated_step(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Continue {
    unsafe {
        enter_native(e, r, pc, target, start, argc);
        let base = r.offset_from(e.vm.stack.as_ptr()) as usize;
        let reader = matches!(prim::name(target), Some("read" | "read-from-string"));
        let saved_depth = e.vm.eval_depth;
        let mut roots = e.vm.external_roots.len();
        if reader {
            roots = e.suspend_code();
            e.vm.eval_depth = e.depth_base + e.frames.len();
            e.vm.propagated_escape = None;
            e.vm.propagated_escape_hooked = false;
            e.vm.propagated_eval_depth = None;
        }
        let step = e.vm.call_generated(target, base + start, argc);
        if !reader {
            return take_step(e, r, pc, step);
        }
        // Rewrite in place: suspended PCs and entry pointers must not move.
        e.resume_code(roots);
        e.vm.eval_depth = saved_depth;
        e.fatal_image_failure |= e.vm.execution_fatal;
        e.code.resize_with(e.vm.functions.len(), Vec::new);
        let width = e.vm.register_width;
        let Some(r) = widen_registers(e, r, width) else {
            fail(e, "register storage overflow");
            return Continue::Stop;
        };
        if let Some(escape) = e.vm.propagated_escape.take() {
            e.escape = Some(escape);
            e.escape_hooked = std::mem::take(&mut e.vm.propagated_escape_hooked);
            e.failed_eval_depth = e.vm.propagated_eval_depth.take();
            return Continue::Stop;
        }
        take_step(e, r, pc, step)
    }
}
/// Act on a Step produced at `pc`: a call site, or `K_CODE[1]` when a
/// continuation ran in its own activation (which a further Call then reuses).
#[inline(never)]
unsafe fn take_step(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    step: std::result::Result<prim::Step, Option<(Value, Value)>>,
) -> Continue {
    unsafe {
        let mut handlers = None;
        let (f, args, then, saved) = match step {
            Err(Some((condition, data))) => {
                e.escape = Some(Escape::Signal(condition, data));
                return Continue::Stop;
            }
            // A watcher's escape, its handler search already done.
            Err(None) => {
                e.escape = e.vm.propagated_escape.take();
                e.escape_hooked = std::mem::take(&mut e.vm.propagated_escape_hooked);
                e.failed_eval_depth = e.vm.propagated_eval_depth.take();
                return Continue::Stop;
            }
            Ok(prim::Step::Collect) => {
                leave_native(e);
                if e.vm.gc_threshold == usize::MAX {
                    // Nested compilation/load owns unforwarded Rust locals.
                    // Match GNU's garbage_collection_inhibited early return.
                    *r.add((*pc).a) = Value::NIL;
                } else {
                    poll_collect(e);
                    *r.add((*pc).a) = e.vm.garbage_collect_statistics();
                }
                return Continue::At(r, pc.add(1));
            }
            Ok(prim::Step::Done(value)) => {
                leave_native(e);
                if std::ptr::eq(pc, &K_CODE[1]) && e.controls.len() > e.controls_base
                    && e.controls.last().is_some_and(|c| matches!(c.kind, ControlKind::HandlerBind { .. })) {
                    let control = e.controls.pop().unwrap();
                    release_control(e, &control);
                }
                *r.add((*pc).a) = value;
                return Continue::At(r, pc.add(1));
            }
            Ok(prim::Step::Call { f, args, then, saved }) => (f, args, then, saved),
            Ok(prim::Step::Handle { body, handlers: bindings }) => {
                handlers = Some(bindings);
                (body, Vec::new(), handler_bind_return as prim::Then, Vec::new())
            }
        };
        let resuming = std::ptr::eq(pc, &K_CODE[1]);
        let (k, depth) = if resuming {
            (r, e.frames.len())
        } else {
            if !depth_room(e) { return Continue::Stop; }
            let width = if std::ptr::eq(pc, &K_CODE[0]) {
                k_width(r)
            } else {
                e.vm.functions.get_unchecked(e.function).regs.max(1)
            };
            (r.add(width), e.frames.len() + 1)
        };
        // Frames assume an activation at depth d starts within (d + 1) register
        // widths; a callee of this one starts at k + width at depth + 1. This
        // is conservative; widen/grow the storage below before writing any
        // activation wider than the compiled register width.
        let width = K_SAVED + saved.len() + args.len();
        let offset = k.offset_from(e.vm.stack.as_ptr()) as usize;
        let fits = |e: &Engine<'_>| {
            (depth + 2)
                .checked_mul(e.vm.register_width)
                .zip(offset.checked_add(width))
                .is_some_and(|(limit, end)| end <= limit && end <= e.vm.stack.len() - 4096)
        };
        // The register width is the widest compiled function so far, so a
        // request wider than every function compiled yet (the first T2 call
        // of a fresh VM whose only form is a tiny top-level one) fails the
        // bound at a shallow depth. That is not depth: widen the registers
        // until the activation fits, rebasing every frame exactly as
        // `force_lazy` does when compiling widens them. Lisp depth is checked
        // separately against the dynamically bound variable above.
        let r = if fits(e) {
            r
        } else {
            let Some(rebased) = offset
                .checked_add(width)
                .and_then(|end| widen_registers(e, r, end.div_ceil(depth + 2)))
            else {
                fail(e, "register storage exhausted");
                return Continue::Stop;
            };
            rebased
        };
        let k = e.vm.stack.as_mut_ptr().add(offset);
        if !fits(e) {
            fail(e, "register storage exhausted");
            return Continue::Stop;
        }
        if !resuming {
            e.call_marks.push(e.native_mark.take());
            e.rest.activations.push((e.rest.values.len(), None));
            e.frames.push(Frame {
                pc: pc.add(1),
                regs: r,
                dest: (*pc).a,
                function: e.function,
                dynamic: e.dynamic,
                controls_base: e.controls_base,
            });
            e.controls_base = e.controls.len();
            e.regs = k;
            e.dynamic = e.vm.dynamic.len();
        }
        let index = match e
            .continuations
            .iter()
            .position(|&known| std::ptr::fn_addr_eq(known, then))
        {
            Some(index) => index,
            None => {
                e.continuations.push(then);
                e.continuations.len() - 1
            }
        };
        let count = |n: usize| Value::integer(n as i64).expect("register count");
        *k.add(K_RES) = Value::NIL;
        *k.add(K_THEN) = count(index);
        *k.add(K_F) = f;
        *k.add(K_SAVED_N) = count(saved.len());
        *k.add(K_ARGS_N) = count(args.len());
        std::ptr::copy_nonoverlapping(saved.as_ptr(), k.add(K_SAVED), saved.len());
        std::ptr::copy_nonoverlapping(args.as_ptr(), k.add(K_SAVED + saved.len()), args.len());
        e.stack_used = e.stack_used.max(offset + width);
        if let Some(handlers) = handlers {
            let mark = e.vm.signal_handlers.len();
            for (conditions, function) in handlers.into_iter().rev() {
                e.vm.signal_handlers.push(SignalHandler::Bind { conditions, function, group: mark });
            }
            e.controls.push(Control { kind: ControlKind::HandlerBind { mark },
                dest: 0, target: 0, dynamic: e.vm.dynamic.len(), registry: None });
        }
        // Every Value of the request now lives in registers or traced handler storage.
        e.budget -= 1;
        if e.budget < 0 {
            Continue::Poll(k, K_CODE.as_ptr())
        } else {
            Continue::At(k, K_CODE.as_ptr())
        }
    }
}
fn handler_bind_return(_: &mut prim::Leaf<'_>, value: Value, _: &[Value]) -> prim::Next {
    Ok(prim::Step::Done(value))
}

/// Run the continuation of the activation at `k` on K_RES and its saved
/// registers, copied for the duration of a leaf call that cannot collect.
#[inline(never)]
unsafe fn resume_step(e: &mut Engine<'_>, k: *mut Value, pc: *const Inst) -> Continue {
    unsafe {
        let result = *k.add(K_RES);
        let then = e.continuations[k_count(*k.add(K_THEN))];
        let n = k_count(*k.add(K_SAVED_N));
        let saved = std::slice::from_raw_parts(k.add(K_SAVED), n).to_vec();
        let step = e.vm.resume_generated(then, result, &saved);
        drop(saved);
        take_step(e, k, pc, step)
    }
}
/// `K_CODE[0]`: call K_F on the argument registers.
unsafe fn t2_call(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let saved = k_count(*r.add(K_SAVED_N));
        let argc = k_count(*r.add(K_ARGS_N));
        t2_dispatch(e, r, pc, *r.add(K_F), K_SAVED + saved, argc)
    }
}
/// `K_CODE[1]`: run the continuation on K_RES and the saved registers.
unsafe fn t2_resume(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let to = resume_step(e, r, pc);
        continue_at(e, to)
    }
}
/// Ordinary call semantics from the continuation activation. It mirrors
/// `invoke_full` but never touches the site caches, and places a callee frame
/// after the activation's own registers rather than the caller function's.
#[cold]
#[inline(never)]
unsafe fn t2_dispatch(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        let function = target;
        let Some(target) = t2_resolve(e, target) else {
            return Value::UNBOUND;
        };
        if target.tag() == 3 && e.vm.autoload_object(target) {
            enter_native(e, r, pc, target, start, argc);
            return match autoload_then(e, r, function, target) {
                Some(r) => t2_dispatch(e, r, pc, function, start, argc),
                None => Value::UNBOUND,
            };
        }
        if lambda_list(e, target) {
            enter_native(e, r, pc, target, start, argc);
            if let Some(failure) = lambda_list_failure(e.vm, target, std::slice::from_raw_parts(r.add(start), argc).to_vec()) {
                return lambda_list_raise(e, failure);
            }
            return match lambda_list_then(e, r, target) {
                Some((r, closure)) => t2_dispatch(e, r, pc, closure, start, argc),
                None => Value::UNBOUND,
            };
        }
        if target.tag() == 4 {
            return t2_enter_closure(e, r, pc, target, start, argc);
        }
        if prim::generated(target).is_some() {
            return enter_generated(e, r, pc, target, start, argc);
        }
        if target.tag() != 6 {
            return {
                let condition = uncallable_condition(e, target);
                raise(e, condition, &[function])
            };
        }
        let Some(p) = prim::legacy(target) else {
            return fail(e, "invalid primitive reference");
        };
        let base = r.offset_from(e.vm.stack.as_ptr()) as usize;
        match p {
            Primitive::Apply => {
                enter_apply(e, r, pc, start, argc);
                let Some((target, offset, count)) = t2_prepare_apply(e, base + start, argc) else {
                    return Value::UNBOUND;
                };
                e.apply_function = Some(target);
                t2_dispatch(e, r, pc, target, offset - base, count)
            }
            // These use neither the site caches nor the caller's width.
            Primitive::Throw | Primitive::Signal | Primitive::Load | Primitive::Require | Primitive::Eval | Primitive::Macroexpand | Primitive::AutoloadDoLoad | Primitive::ReadHostEvent | Primitive::FormatModeLine | Primitive::MakeInterpretedClosure => {
                t2_invoke_full(e, r, pc, target, start, argc)
            }
            _ => {
                enter_native(e, r, pc, target, start, argc);
                let Some(value) = primitive_value(e, p, base + start, argc, primitive_target(p))
                else {
                    return Value::UNBOUND;
                };
                leave_native(e);
                *r.add((*pc).a) = value;
                advance(e, r, pc)
            }
        }
    }
}
// t2_dispatch must stay free of address-taken locals: one escaping alloca stops
// LLVM marking any of its calls `tail`, so every callback through it would keep
// a native frame. Without LTO, as under the gate profile, `HashSet::contains`
// is not inlined and takes `&slot`. The helpers below keep that scratch, and
// the prepare_apply Result with its error drop, out of the dispatching frame.
/// Follow symbol function indirections; `None` after recording the fault.
#[inline(never)]
fn t2_resolve(e: &mut Engine<'_>, target: Value) -> Option<Value> {
    let mut target = target;
    let mut aliases = 0;
    while target.tag() == 7 && target != Value::UNBOUND {
        let slot = target.payload() as usize - 1;
        if e.vm.macros.contains(&slot) && !e.vm.expanding.contains(&slot) {
            fail(e, &format!("invalid function: macro {} is not callable", e.vm.symbol_names[slot]));
            return None;
        }
        let Some(entry) = e.vm.slots.get(slot) else {
            fail(e, "invalid symbol");
            return None;
        };
        target = entry.function;
        aliases += 1;
        if aliases > e.vm.slots.len() {
            fail(e, "cyclic function indirection");
            return None;
        }
    }
    Some(target)
}
/// `invoke_full` out of line, so its alias scratch never joins t2_dispatch.
#[cold]
#[inline(never)]
unsafe fn t2_invoke_full(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    unsafe { invoke_full(e, r, pc, target, start, argc) }
}
/// Spread `apply`'s arguments; `None` after recording the error.
#[cold]
#[inline(never)]
fn t2_prepare_apply(
    e: &mut Engine<'_>,
    start: usize,
    argc: usize,
) -> Option<(Value, usize, usize)> {
    // prepare_apply writes a disjoint scratch window at the end.
    match e.vm.prepare_apply(start, argc) {
        Ok(window) => {
            match e.apply_scratch.iter_mut().find(|(offset, _)| *offset == window.1) {
                Some((_, count)) => *count = (*count).max(window.2),
                None => e.apply_scratch.push((window.1, window.2)),
            }
            Some(window)
        }
        Err(error) => {
            e.error = Some(error);
            None
        }
    }
}
/// `invoke`'s general closure entry, with the callee after the activation.
#[cold]
#[inline(never)]
unsafe fn t2_enter_closure(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        let Some(closure) = e.vm.closures.get(target.payload() as usize) else {
            return fail(e, "invalid closure reference");
        };
        let function = closure.function;
        if !depth_room(e) { return Value::UNBOUND; }
        let Some(r) = frame_room(e, r) else { return fail(e, "register storage overflow"); };
        let mut r = r;
        if e.vm.functions[function].lazy.is_some() {
            match force_lazy(e, r, function) {
                Some(rebased) => r = rebased,
                None => return Value::UNBOUND,
            }
        }
        if let Err(error) = lower_function(
            e.vm,
            &mut e.code,
            &mut e.entries,
            &mut e.inline_bodies,
            &mut e.free_entries,
            function,
        ) {
            e.fatal_image_failure = true;
            e.error = Some(error);
            return Value::UNBOUND;
        }
        let closure = &e.vm.closures[target.payload() as usize];
        let f = &e.vm.functions[function];
        if argc < f.required || (!f.rest && argc > f.arity) {
            enter_native(e, r, pc, target, start, argc);
            return closure_arity_error(e, function, argc);
        }
        let new = r.add(k_width(r));
        let dst = new.add(f.captures);
        let positional = argc.min(f.arity);
        // As in the cached entry: read every argument, the backtrace's
        // included, before writing the frame, which may overlap `apply`'s
        // spread window after widening.
        let mark = call_mark(e, r, pc, target, start, argc);
        let f = &e.vm.functions[function];
        let closure = &e.vm.closures[target.payload() as usize];
        let rest = if f.rest {
            let begin = e.rest.values.len();
            for i in positional..argc {
                e.rest.values.push(*r.add(start + i));
            }
            Some(OpaqueRest {
                reg: f.captures + f.arity,
                start: begin,
                end: e.rest.values.len(),
            })
        } else {
            None
        };
        std::ptr::copy(r.add(start), dst, positional);
        std::ptr::copy_nonoverlapping(closure.captures.as_ptr(), new, closure.captures.len());
        for i in positional..f.arity {
            *dst.add(i) = Value::NIL;
        }
        if f.rest {
            *dst.add(f.arity) = Value::NIL;
        }
        enter_frame_marked(e, r, pc, new, function, rest, mark)
    }
}
// Resolve the ordinary primitive family once per call site. Each cached call
// enters its Rust kernel directly instead of walking Vm::primitive's chain.
fn primitive_target(p: Primitive) -> PrimitiveCall {
    use Primitive::*;
    match p {
        ClosureMetadata | ClosureDocument | MakeInterpretedClosure => Vm::interpreted_closure_primitive,
        ForwardChar | BackwardChar | BeginningOfLine | EndOfLine => {
            Vm::command_primitive
        }
        TempBuffer => Vm::eval_buffer_primitive,
        DeepEqual | Min | Append | Prin1ToString => Vm::eval_data_primitive,
        GetBuffer | ForwardLine | SwitchToBuffer | EmacsLispMode | MenuBarMode
        | ForceModeLineUpdate | LocateLibrary | Daemonp | DaemonInitialized => {
            Vm::eval_buffer_primitive
        }
        p if (p as usize) >= SymbolValue as usize => Vm::variable_primitive,
        MakeObsolete | MakeObsoleteVariable | SetAdvertisedCallingConvention => {
            Vm::metadata_primitive
        }
        Fset => Vm::bootstrap_primitive,
        Consp | CarSafe | Symbolp | Integerp | Fixnump | Bignump => Vm::surface_primitive,
        // Never invoked: `primitive_value` routes these through their signal adapter.
        p if hash::is_hash_primitive(p) => Vm::hash_primitive_result,
        p if regexp::is_regexp_primitive(p) => Vm::regexp_primitive_result,
        p if (p as usize) >= DefineKey as usize => Vm::keymap_primitive,
        p if (p as usize) >= DisplayDefineFace as usize => Vm::display_primitive,
        Add => Vm::add_primitive,
        Sub | Mul | Div | Abs | Less | Equal | Inc | Dec => Vm::numeric_primitive,
        Vector | Aref | Aset | CopySequence => Vm::data_primitive,
        CacheStats | CacheClear | CacheEntries => |vm, p, _, n| vm.cache_primitive(p, n),
        ProcessLines => |vm, _, start, n| vm.process_lines(start, n),
        Format => |vm, _, start, n| vm.format_string(start, n),
        // Traversal and Message retain their ordinary arity/error adapter.
        Car | Cdr | Null | Eq | Memq | Assq | PlistGet | Nth | Setcar | Setcdr => {
            Vm::traversal_primitive
        }
        Message => Vm::primitive,
        Load | Require | Provide | Featurep | Eval | Macroexpand | AutoloadDoLoad | ReadHostEvent | FormatModeLine => |vm, p, start, n| {
            let args = vm.stack[start..start + n].to_vec();
            vm.loader_primitive(p, &args)
        },
        Get | Put | RawPropertiesAt | GetCharProperty | BufferSubstring
        | InsertInherit | KillNew | CurrentKill | YankPart | YankFinalize | YankPropertyRun
        | YankFontLockFace | YankCategory | GetProperty => Vm::annotation_primitive,
        p if (List as usize..=Memq as usize).contains(&(p as usize)) => Vm::bootstrap_primitive,
        p if (p as usize) >= Remainder as usize => Vm::buffer_primitive,
        _ => Vm::primitive,
    }
}
// The Result out-pointer and diagnostic allocations stay in this non-threaded
// helper, so no escaped native stack address prevents the next tail dispatch.
#[inline(never)]
fn primitive_value(
    e: &mut Engine<'_>,
    p: Primitive,
    start: usize,
    argc: usize,
    call: PrimitiveCall,
) -> Option<Value> {
    // `error` signals a typed condition. It must not travel the internal string
    // channel: `route_escape`'s text fallback classifies VM-raised faults by
    // their wording, so a user's message chose the condition -- `(error "x
    // required")` was caught as `wrong-type-argument` -- and the data was lost
    // because that fallback signals NIL. GNU `src/eval.c` `Ferror` signals
    // `Qerror` with the formatted message in the data list.
    //
    // Both dispatchers funnel through this adapter, so intercepting here covers
    // the threaded path and `Vm::primitive`. No collection can occur between the
    // allocations below: `list_value` only pushes, and collection is request
    // driven, which is the same sequencing `condition_call` and `hash_call` rely
    // on.
    if (Primitive::ForwardChar as usize..=Primitive::EndOfLine as usize)
        .contains(&(p as usize))
    {
        let mut signal = None;
        let result = e.vm.command_call(p, start, argc, &mut signal);
        if let Some((condition, data)) = signal {
            e.escape = Some(Escape::Signal(condition, data));
        }
        return match result {
            Ok(value) => Some(value),
            Err(error) => {
                e.error = Some(error);
                None
            }
        };
    }
    if p == Primitive::DisplayError {
        if argc < 1 {
            e.error = Some("wrong primitive arity".into());
            return None;
        }
        let message = match e.vm.format_string(start, argc) {
            Ok(message) => message,
            Err(fault) => {
                e.error = Some(fault);
                return None;
            }
        };
        // Taken before the data list is built, for the unhandled-signal
        // diagnostic, which still reports the author's text.
        e.error = e.vm.source_string(message).ok().map(|s| s.to_owned());
        let condition = e.vm.intern("error");
        let data = e.vm.list_value(&[message]);
        e.escape = Some(Escape::Signal(condition, data));
        return None;
    }
    if matches!(
        p,
        Primitive::DefineError | Primitive::ErrorMessage | Primitive::Get | Primitive::Put
    ) || hash::is_hash_primitive(p)
        || regexp::is_regexp_primitive(p)
    {
        let mut signal = None;
        let result = if regexp::is_regexp_primitive(p) {
            e.vm.regexp_call(p, start, argc, &mut e.error, &mut signal)
        } else if hash::is_hash_primitive(p) {
            e.vm.hash_call(p, start, argc, &mut e.error, &mut signal)
        } else {
            e.vm.condition_call(p, start, argc, &mut e.error, &mut signal)
        };
        if let Some((condition, data)) = signal {
            e.escape = Some(Escape::Signal(condition, data));
        }
        return result;
    }
    match call(e.vm, p, start, argc) {
        Ok(value) => Some(value),
        Err(error) => {
            // A variable watcher's or `defalias-fset-function''s own signal
            // travels behind the write's text.
            if matches!(p, Primitive::Set | Primitive::Makunbound | Primitive::DefvarSetDefault | Primitive::Defalias) && let Some(escape) = e.vm.propagated_escape.take() {
                e.escape = Some(escape);
                e.escape_hooked = std::mem::take(&mut e.vm.propagated_escape_hooked);
                e.failed_eval_depth = e.vm.propagated_eval_depth.take();
            } else if let Some((condition, data)) = e.vm.primitive_condition(p, start, argc, &error) {
                e.escape = Some(Escape::Signal(condition, data));
            }
            e.error = Some(error);
            None
        }
    }
}
/// A primitive through the site's entry cache, which holds its call. Six
/// register arguments: a seventh on the stack would stop every caller from
/// tail-calling it, so each funcall or apply of a primitive kept a frame.
unsafe fn cached_primitive(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        let call = e.entries.get_unchecked((*pc).site).primitive.expect("cached primitive call");
        enter_native(e, r, pc, target, start, argc);
        let base = r.offset_from(e.vm.stack.as_ptr()) as usize;
        let p = prim::legacy(target).expect("legacy primitive");
        let Some(value) = primitive_value(e, p, base + start, argc, call) else {
            return Value::UNBOUND;
        };
        leave_native(e);
        *r.add((*pc).a) = value;
        advance(e, r, pc)
    }
}
#[cold]
unsafe fn primitive(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        enter_native(e, r, pc, target, start, argc);
        let base = r.offset_from(e.vm.stack.as_ptr()) as usize;
        let p = prim::legacy(target).expect("legacy primitive");
        if matches!(p, Primitive::Load | Primitive::Require | Primitive::Eval | Primitive::Macroexpand | Primitive::AutoloadDoLoad | Primitive::ReadHostEvent | Primitive::FormatModeLine | Primitive::MakeInterpretedClosure) {
            return reentrant_load(e, r, pc, p, base + start, argc);
        }
        let mut signal = None;
        match e
            .vm
            .primitive_call(p, base + start, argc, &mut e.error, &mut signal)
        {
            Some(value) => {
                leave_native(e);
                *r.add((*pc).a) = value;
                advance(e, r, pc)
            }
            None => {
                if let Some((condition, data)) = signal {
                    e.escape = Some(Escape::Signal(condition, data));
                }
                Value::UNBOUND
            }
        }
    }
}
/// Raise the register width to at least `width` and grow the shared register
/// stack to match, rebasing every frame pointer and `e.regs` the way
/// `force_lazy` does after compiling widens the registers. Nothing runs Lisp
/// or allocates Lisp objects here, so no collection can observe the move.
/// Returns `r` rebased, or None when the storage size overflows.
#[cold]
#[inline(never)]
unsafe fn widen_registers(e: &mut Engine<'_>, r: *mut Value, width: usize) -> Option<*mut Value> {
    unsafe {
        let width = width.max(e.vm.register_width);
        let frames = (e.frames.len() + 3).max(MIN_FRAMES).checked_next_power_of_two()?;
        let size = width.checked_mul(frames)?.checked_add(4096)?;
        let old_base = e.vm.stack.as_ptr();
        let current = r.offset_from(old_base) as usize;
        let regs = e.regs.offset_from(old_base) as usize;
        let frames: Vec<_> = e
            .frames
            .iter()
            .map(|frame| frame.regs.offset_from(old_base) as usize)
            .collect();
        e.vm.register_width = width;
        if e.vm.stack.len() < size {
            e.vm.stack.resize(size, Value::NIL);
        }
        let base = e.vm.stack.as_mut_ptr();
        for (i, offset) in frames.into_iter().enumerate() {
            e.frames[i].regs = base.add(offset);
        }
        e.regs = base.add(regs);
        Some(base.add(current))
    }
}
/// R215: expand and compile a lazily held body before its first frame entry.
/// Exactly `reentrant_load`'s discipline: macro expansion runs Lisp, so the
/// suspended register stack is taken out of the collector's view, collection
/// is inhibited while compiler builders hold unrooted Values, and every frame
/// pointer is rebased afterwards because compiling can widen the registers.
/// Returns the rebased caller registers, or None with the expansion's own
/// condition pending (GNU signals it when the call reaches the body).
#[cold]
#[inline(never)]
unsafe fn force_lazy(e: &mut Engine<'_>, r: *mut Value, function: usize) -> Option<*mut Value> {
    unsafe {
        let old_base = e.vm.stack.as_ptr();
        let current = r.offset_from(old_base) as usize;
        let frames: Vec<_> = e
            .frames
            .iter()
            .map(|frame| frame.regs.offset_from(old_base) as usize)
            .collect();
        let stack = std::mem::take(&mut e.vm.stack);
        let external = std::mem::take(&mut e.vm.external_roots);
        let threshold = e.vm.gc_threshold;
        e.vm.gc_threshold = usize::MAX;
        e.vm.execution_fatal = false;
        e.vm.propagated_escape = None;
        e.vm.propagated_eval_depth = None;
        let saved_depth = e.vm.eval_depth;
        e.vm.eval_depth = e.depth_base + e.frames.len();
        let result = stacker::maybe_grow(512 * 1024, 2 * 1024 * 1024, || e.vm.force_lazy(function));
        e.vm.eval_depth = saved_depth;
        e.fatal_image_failure |= e.vm.execution_fatal;
        e.vm.gc_threshold = threshold;
        e.vm.stack = stack;
        e.vm.external_roots = external;
        e.vm.end_nested_execution();
        e.code.resize_with(e.vm.functions.len(), Vec::new);
        let width = e.vm.register_width;
        let Some(size) = width.checked_mul((e.frames.len() + 3).max(MIN_FRAMES)).and_then(|n| n.checked_add(4096)) else {
            fail(e, "register storage overflow");
            return None;
        };
        if e.vm.stack.len() < size {
            e.vm.stack.resize(size, Value::NIL);
        }
        let base = e.vm.stack.as_mut_ptr();
        for (i, offset) in frames.into_iter().enumerate() {
            e.frames[i].regs = base.add(offset);
        }
        let r = base.add(current);
        e.regs = r;
        match result {
            Ok(()) => Some(r),
            Err(error) => {
                if let Some(escape) = e.vm.propagated_escape.take() {
                    e.escape = Some(escape);
                    e.escape_hooked = std::mem::take(&mut e.vm.propagated_escape_hooked);
                    e.failed_eval_depth = e.vm.propagated_eval_depth.take();
                } else {
                    let message = e.vm.literal(&Form::String(error));
                    match message {
                        Ok(message) => {
                            raise(e, "error", &[message]);
                        }
                        Err(error) => e.error = Some(error),
                    }
                }
                None
            }
        }
    }
}
/// eval.c `Fautoload_do_load (FUNDEF, FUNCTION, Qnil)` for a call site.
/// Returns the rebased caller registers, or None with the load's own
/// condition pending.
#[cold]
#[inline(never)]
unsafe fn autoload_then(e: &mut Engine<'_>, r: *mut Value, function: Value, fundef: Value) -> Option<*mut Value> {
    unsafe { nested_then(e, r, |vm| vm.autoload_do_load(fundef, function, Value::NIL)).map(|(r, _)| r) }
}
/// eval.c funcall_lambda of a `(lambda ARGS . BODY)' list: GNU interprets
/// it with dynamic binding (lexenv nil). VM2 evaluates the list in that
/// dialect, as `(function LIST)', and calls the closure; the list itself
/// stays the function. Returns the rebased caller registers and the
/// closure, or None with the condition pending.
#[cold]
#[inline(never)]
unsafe fn lambda_list_then(e: &mut Engine<'_>, r: *mut Value, list: Value) -> Option<(*mut Value, Value)> {
    unsafe { nested_then(e, r, |vm| vm.eval_primitive(&[list, Value::NIL])) }
}
/// A lambda list's failed walk: the bindings made before it failed,
/// then the condition and its data.
type LambdaListFailure = (Vec<(Value, Value)>, &'static str, Vec<Value>);
/// eval.c funcall_lambda's argument walk for a `(lambda ARGS . BODY)'
/// list called with ARGS. One parameter at a time it is checked, takes its
/// argument and is bound (dynamically: a list function has no lexenv). A
/// parameter that is not a symbol or a misplaced &optional or &rest is
/// invalid-function, binding nil, t or a keyword is setting-constant, and
/// a missing or excess argument is wrong-number-of-arguments, with the list
/// itself as the function datum. Returns the walk's failure, if any.
fn lambda_list_failure(vm: &mut Vm, list: Value, args: Vec<Value>) -> Option<LambdaListFailure> {
    let symbol = |vm: &mut Vm, name: &str| Value::reference(vm.slot(name) + 1, 7);
    let (and_rest, and_optional) = (symbol(vm, "&rest"), symbol(vm, "&optional"));
    let constant = |vm: &Vm, param: Value| param == Value::NIL || param == Value::TRUE || {
        let slot = param.index_if(7).map(|i| i - 1);
        slot.is_some_and(|slot| vm.symbol_names[slot].starts_with(':') && vm.names.get(&vm.symbol_names[slot]) == Some(&slot))
    };
    let (_, tail) = vm.cons_pair(list).ok()?;
    let (mut params, _) = vm.cons_pair(tail).ok()?;
    let argc = args.len();
    let invalid = Some(("invalid-function", vec![list]));
    let arity = Some(("wrong-number-of-arguments", vec![list, Value::integer(argc as i64).unwrap()]));
    let (mut bindings, mut optional, mut rest, mut previous_rest, mut i) = (vec![], false, false, false, 0);
    let failure = loop {
        let Ok((next, more)) = vm.cons_pair(params) else {
            break if params != Value::NIL || previous_rest { invalid } else if i < argc { arity } else { None };
        };
        if next.tag() != 7 && next != Value::NIL && next != Value::TRUE { break invalid }
        if next == and_rest {
            if rest || previous_rest { break invalid }
            (rest, previous_rest) = (true, true);
        } else if next == and_optional {
            if optional || rest || previous_rest { break invalid }
            optional = true;
        } else {
            let arg = if rest { let all = vm.list_value(&args[i..]); i = argc; all }
                else if i < argc { i += 1; args[i - 1] }
                else if !optional { break arity }
                else { Value::NIL };
            if constant(vm, next) { break Some(("setting-constant", vec![next])) }
            bindings.push((next, arg));
            previous_rest = false;
        }
        params = more;
    };
    let (condition, data) = failure?;
    Some((bindings, condition, data))
}
/// Make a failed walk's bindings, with the caller's own argument objects,
/// then signal its condition; the handler's unwinding undoes the bindings,
/// so their variable watchers see the let and the unlet. The values stay
/// rooted (and are reread) while watchers run Lisp.
fn lambda_list_raise(e: &mut Engine<'_>, (bindings, condition, data): LambdaListFailure) -> Value {
    let roots = e.vm.external_roots.len();
    e.vm.external_roots.extend(&data);
    e.vm.external_roots.extend(bindings.iter().flat_map(|&(param, arg)| [param, arg]));
    let base = roots + data.len();
    for k in 0..bindings.len() {
        let (param, arg) = (e.vm.external_roots[base + 2 * k], e.vm.external_roots[base + 2 * k + 1]);
        if let Err(error) = e.vm.bind_variable(param.index_if(7).map_or(0, |i| i - 1), arg) {
            e.vm.external_roots.truncate(roots);
            if let Some(escape) = e.vm.propagated_escape.take() {
                e.escape = Some(escape);
            } else if let Some((condition, data)) = e.vm.forwarded_condition(&error, arg) {
                e.escape = Some(Escape::Signal(condition, data));
            } else {
                e.error = Some(error);
            }
            return Value::UNBOUND;
        }
    }
    let data: Vec<Value> = e.vm.external_roots.drain(roots..).take(data.len()).collect();
    raise(e, condition, &data)
}
/// GNU `(lambda ...)' list functions: a list headed by `lambda' whose cdr
/// is a cons (funcall_lambda signals `invalid-function' otherwise).
fn lambda_list(e: &mut Engine<'_>, target: Value) -> bool {
    target.tag() == 3 && {
        let lambda = Value::reference(e.vm.slot("lambda") + 1, 7);
        e.vm.cons_pair(target).is_ok_and(|(head, tail)| head == lambda && tail.tag() == 3)
    }
}
/// Run RUN, which may load and compile Lisp, from a call site:
/// `force_lazy`'s discipline (the suspended register stack leaves the
/// collector's view, collection is inhibited, every frame pointer is rebased
/// because the nested code can widen the registers). Returns the rebased
/// caller registers and RUN's value, or None with its condition pending.
unsafe fn nested_then(e: &mut Engine<'_>, r: *mut Value, run: impl FnOnce(&mut Vm) -> Result<Value>) -> Option<(*mut Value, Value)> {
    unsafe {
        let old_base = e.vm.stack.as_ptr();
        let current = r.offset_from(old_base) as usize;
        let frames: Vec<_> = e
            .frames
            .iter()
            .map(|frame| frame.regs.offset_from(old_base) as usize)
            .collect();
        let stack = std::mem::take(&mut e.vm.stack);
        let external = std::mem::take(&mut e.vm.external_roots);
        let threshold = e.vm.gc_threshold;
        e.vm.gc_threshold = usize::MAX;
        e.vm.execution_fatal = false;
        e.vm.propagated_escape = None;
        e.vm.propagated_eval_depth = None;
        let saved_depth = e.vm.eval_depth;
        e.vm.eval_depth = e.depth_base + e.frames.len();
        e.vm.loader.propagate += 1;
        let result = stacker::maybe_grow(512 * 1024, 2 * 1024 * 1024, || run(e.vm));
        e.vm.loader.propagate -= 1;
        e.vm.eval_depth = saved_depth;
        e.fatal_image_failure |= e.vm.execution_fatal;
        e.vm.gc_threshold = threshold;
        e.vm.stack = stack;
        e.vm.external_roots = external;
        e.vm.end_nested_execution();
        e.code.resize_with(e.vm.functions.len(), Vec::new);
        let width = e.vm.register_width;
        let Some(size) = width.checked_mul((e.frames.len() + 3).max(MIN_FRAMES)).and_then(|n| n.checked_add(4096)) else {
            fail(e, "register storage overflow");
            return None;
        };
        if e.vm.stack.len() < size {
            e.vm.stack.resize(size, Value::NIL);
        }
        let base = e.vm.stack.as_mut_ptr();
        for (i, offset) in frames.into_iter().enumerate() {
            e.frames[i].regs = base.add(offset);
        }
        let r = base.add(current);
        e.regs = r;
        match result {
            Ok(value) => Some((r, value)),
            Err(error) => {
                if let Some(escape) = e.vm.propagated_escape.take() {
                    e.escape = Some(escape);
                    e.escape_hooked = std::mem::take(&mut e.vm.propagated_escape_hooked);
                    e.failed_eval_depth = e.vm.propagated_eval_depth.take();
                } else {
                    e.error = Some(error);
                }
                None
            }
        }
    }
}
// A load can compile and run new functions while an outer frame is suspended.
// Retain that frame's allocation, inhibit compaction, then rebase every pointer
// if the newly compiled register width requires a larger outer stack.
#[cold]
unsafe fn reentrant_load(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    p: Primitive,
    start: usize,
    argc: usize,
) -> Value {
    unsafe {
        let old_base = e.vm.stack.as_ptr();
        let current = r.offset_from(old_base) as usize;
        let frames: Vec<_> = e
            .frames
            .iter()
            .map(|frame| frame.regs.offset_from(old_base) as usize)
            .collect();
        let args = e.vm.stack[start..start + argc].to_vec();
        let stack = std::mem::take(&mut e.vm.stack);
        let external = std::mem::take(&mut e.vm.external_roots);
        let threshold = e.vm.gc_threshold;
        e.vm.gc_threshold = usize::MAX;
        e.vm.execution_fatal = false;
        // Only an escape produced by THIS operation may be re-raised below.
        e.vm.propagated_escape = None;
        e.vm.propagated_eval_depth = None;
        let saved_depth = e.vm.eval_depth;
        e.vm.eval_depth = e.depth_base + e.frames.len();
        e.vm.loader.propagate += 1;
        let result = stacker::maybe_grow(512 * 1024, 2 * 1024 * 1024, || e.vm.loader_primitive(p, &args));
        e.vm.loader.propagate -= 1;
        e.vm.eval_depth = saved_depth;
        e.fatal_image_failure |= e.vm.execution_fatal;
        e.vm.gc_threshold = threshold;
        e.vm.stack = stack;
        e.vm.external_roots = external;
        e.vm.end_nested_execution();
        e.code.resize_with(e.vm.functions.len(), Vec::new);
        let width = e.vm.register_width;
        let Some(size) = width.checked_mul((e.frames.len() + 3).max(MIN_FRAMES)).and_then(|n| n.checked_add(4096)) else {
            return fail(e, "register storage overflow");
        };
        if e.vm.stack.len() < size { e.vm.stack.resize(size, Value::NIL); }
        let base = e.vm.stack.as_mut_ptr();
        for (i, offset) in frames.into_iter().enumerate() {
            e.frames[i].regs = base.add(offset);
        }
        let r = base.add(current);
        e.regs = r;
        match result {
            Ok(value) => {
                leave_native(e);
                *r.add((*pc).a) = value;
                advance(e, r, pc)
            }
            Err(error) => {
                // An `eval` form's own signal or throw crosses back unchanged;
                // nothing has allocated since its engine stored it.
                if let Some(escape) = e.vm.propagated_escape.take() {
                    e.escape = Some(escape);
                    e.escape_hooked = std::mem::take(&mut e.vm.propagated_escape_hooked);
                    e.failed_eval_depth = e.vm.propagated_eval_depth.take();
                    return Value::UNBOUND;
                }
                if error == "native stack exhausted while compiling Lisp" {
                    return native_stack_failure(e, &error);
                }
                if let Some((condition, data)) = e.vm.primitive_condition(p, start, argc, &error) {
                    e.escape = Some(Escape::Signal(condition, data));
                }
                e.error = Some(error);
                Value::UNBOUND
            }
        }
    }
}

macro_rules! numeric {
    ($name:ident,$body:expr) => {
        #[cfg_attr(all(target_os = "linux", target_arch = "x86_64"), unsafe(link_section = concat!(".text.vm2.", stringify!($name))))]
        unsafe fn $name(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
            unsafe {
                let i = &*pc;
                if e.vm.slots.get_unchecked(i.b).function != i.value {
                    return direct(e, r, pc);
                }
                let x = *r.add(i.d);
                let y = if i.e == 2 { *r.add(i.d + 1) } else { Value(5) };
                if x.tag() != 1 || y.tag() != 1 {
                    return direct(e, r, pc);
                }
                let result: Option<Value> = ($body)(x, y);
                match result {
                    Some(v) => {
                        *r.add(i.a) = v;
                        advance(e, r, pc)
                    }
                    None => direct(e, r, pc),
                }
            }
        }
    };
}
numeric!(mul, |x: Value, y: Value| ((x.0 as i64) >> 2)
    .checked_mul((y.0 as i64) >> 2)
    .and_then(|n| Value::fixnum(n)));
numeric!(add, |x: Value, y: Value| (x.0 as i64)
    .checked_add((y.0 - 1) as i64)
    .map(|n| Value(n as u64)));
numeric!(sub, |x: Value, y: Value| (x.0 as i64)
    .checked_sub((y.0 - 1) as i64)
    .map(|n| Value(n as u64)));
numeric!(inc, |x: Value, _y: Value| (x.0 as i64)
    .checked_add(4)
    .map(|n| Value(n as u64)));
numeric!(dec, |x: Value, _y: Value| (x.0 as i64)
    .checked_sub(4)
    .map(|n| Value(n as u64)));
numeric!(less, |x: Value, y: Value| Some(
    if (x.0 as i64) < (y.0 as i64) {
        Value::TRUE
    } else {
        Value::NIL
    }
));
numeric!(equal, |x: Value, y: Value| Some(if x == y {
    Value::TRUE
} else {
    Value::NIL
}));
unsafe fn insert_string(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let i = &*pc;
        if e.vm.slots[i.b].function != i.value {
            return direct(e, r, pc);
        }
        match quiet_insert(e.vm, *r.add(i.d)) {
            None => direct(e, r, pc),
            Some(false) => fail(e, "an insertion's undo record or deactivate-mark failed"),
            Some(true) => {
                *r.add(i.a) = Value::NIL;
                advance(e, r, pc)
            }
        }
    }
}
/// `insert_string''s insertion of string S at the end of the current
/// buffer when it runs no Lisp: None for the general call, Some(false) if
/// its undo record or mark deactivation failed. Kept out of line, as
/// `buffer_read_value' is, so the handler's dispatch stays a tail call.
#[inline(never)]
fn quiet_insert(vm: &mut Vm, s: Value) -> Option<bool> {
    let t = vm.slots[vm.buffer_slot].value;
    if s == Value::NIL || s.tag() != 0 || t == Value::NIL || t.tag() != 0 || s == t {
        return None;
    }
    let a = s.payload() as usize - 1;
    let b = t.payload() as usize - 1;
    if a.max(b) >= vm.objects.len() {
        return None;
    }
    // Only an insertion that runs no Lisp skips the general bracket.
    let quiet = vm.quiet_insertion()?;
    let (source, target) = if a < b {
        let (x, y) = vm.objects.split_at_mut(b);
        (&x[a], &mut y[0])
    } else {
        let (x, y) = vm.objects.split_at_mut(a);
        (&y[0], &mut x[b])
    };
    let (data::Object::String(chars), data::Object::Buffer(text)) = (source, target) else {
        return None;
    };
    let length = chars.len();
    if !chars.properties.is_empty() || !text.append_utf8(chars.as_str()) {
        return None;
    }
    if vm.quiet_inserted(quiet, length).is_err() {
        return Some(false);
    }
    vm.display_state.version += 1;
    Some(true)
}
unsafe fn remainder(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let i = &*pc;
        if e.vm.slots[i.b].function != i.value {
            return direct(e, r, pc);
        }
        let x = *r.add(i.d);
        let y = *r.add(i.d + 1);
        if x.tag() != 1 || y.tag() != 1 || y.0 as i64 >> 2 == 0 {
            return direct(e, r, pc);
        }
        *r.add(i.a) = Value::fixnum((x.0 as i64 >> 2) % (y.0 as i64 >> 2)).unwrap();
        advance(e, r, pc)
    }
}
unsafe fn sequence_read(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let i = &*pc;
        if e.vm.slots[i.b].function != i.value {
            return direct(e, r, pc);
        }
        let x = *r.add(i.d);
        if x == Value::NIL || x.tag() != 0 {
            return direct(e, r, pc);
        }
        let Some(object) = e.vm.objects.get(x.payload() as usize - 1) else {
            return direct(e, r, pc);
        };
        let result = if i.e == 1 {
            match object {
                data::Object::String(s) | data::Object::RawString(s) => Value::fixnum(s.len() as i64),
                data::Object::Vector(s) | data::Object::Record(s) => Value::fixnum(s.len() as i64),
                _ => None,
            }
        } else {
            let y = *r.add(i.d + 1);
            if y.tag() != 1 {
                return direct(e, r, pc);
            }
            let at = (y.0 as i64 >> 2) as usize;
            match object {
                data::Object::String(s) | data::Object::RawString(s) => s
                    .char_code(coordinates::CharOffset(at))
                    .and_then(|c| Value::fixnum(c as i64)),
                data::Object::Vector(s) | data::Object::Record(s) => s.get(at).copied(),
                _ => None,
            }
        };
        let Some(value) = result else {
            return direct(e, r, pc);
        };
        *r.add(i.a) = value;
        advance(e, r, pc)
    }
}
unsafe fn buffer_read(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let i = &*pc;
        if e.vm.slots[i.b].function != i.value {
            return direct(e, r, pc);
        }
        let x = if i.e == 0 { Value::NIL } else { *r.add(i.d) };
        let Some(value) = buffer_read_value(e.vm, i.value, x) else {
            return direct(e, r, pc);
        };
        *r.add(i.a) = value;
        advance(e, r, pc)
    }
}
/// The fast buffer read of `buffer_read' (char-after, goto-char, point,
/// point-min, point-max, buffer-size) with argument X, or None for the
/// general call. Kept out of line: its locals would otherwise stay live
/// across the handler's dispatch to the next instruction, which then
/// cannot be a tail call, and every loop iteration keeps a native frame.
#[inline(never)]
fn buffer_read_value(vm: &mut Vm, target: Value, x: Value) -> Option<Value> {
    let v = vm.slots[vm.buffer_slot].value;
    if v == Value::NIL || v.tag() != 0 {
        return None;
    }
    let Some(data::Object::Buffer(t)) = vm.objects.get_mut(v.payload() as usize - 1) else {
        return None;
    };
    let kind = (target.0 >> 3) as usize;
    let accessible = t.accessible();
    let number = if kind == Primitive::CharAfter as usize {
        if x != Value::NIL
            && x.tag() == 1
            && x.0 as i64 >> 2 >= 1
            && accessible.start.0 == 0
            && accessible.end.0 == t.len_bytes()
        {
            let ch = t.character_of_char(coordinates::CharOffset((x.0 as i64 >> 2) as usize - 1)).ok()?;
            return Some(ch.map(|n| Value::fixnum(n as i64).unwrap()).unwrap_or(Value::NIL));
        }
        let p = if x == Value::NIL {
            t.point
        } else {
            if x.tag() != 1 || x.0 as i64 >> 2 < 1 {
                return None;
            }
            t.byte_of_char(coordinates::CharOffset((x.0 as i64 >> 2) as usize - 1)).ok()?
        };
        if accessible.contains(&p) {
            t.character_at(p).unwrap()
        } else {
            None
        }
    } else if kind == Primitive::GotoChar as usize {
        if x.tag() != 1 {
            return None;
        }
        let c = (x.0 as i64 >> 2).saturating_sub(1).max(0) as usize;
        t.point = t
            .byte_of_char(coordinates::CharOffset(c.min(t.len_chars())))
            .unwrap()
            .clamp(accessible.start, accessible.end);
        Some(x.0 as i64 >> 2)
    } else if kind == Primitive::Point as usize {
        Some((t.char_of_byte(t.point).unwrap().0 + 1) as i64)
    } else if kind == Primitive::PointMin as usize {
        Some((t.char_of_byte(accessible.start).unwrap().0 + 1) as i64)
    } else if kind == Primitive::PointMax as usize {
        Some((t.char_of_byte(accessible.end).unwrap().0 + 1) as i64)
    } else {
        Some(t.len_chars() as i64)
    };
    Some(number.map(|n| Value::fixnum(n).unwrap()).unwrap_or(Value::NIL))
}
unsafe fn cons(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let i = &*pc;
        if e.vm.slots.get_unchecked(i.b).function != i.value {
            return direct(e, r, pc);
        }
        let index = e.vm.conses.len();
        if index == e.vm.conses.capacity() {
            return grow_cons(e, r, pc);
        }
        e.vm.conses
            .as_mut_ptr()
            .add(index)
            .write((*r.add(i.d), *r.add(i.d + 1)));
        e.vm.conses.set_len(index + 1);
        *r.add(i.a) = Value::reference(index, 3);
        advance(e, r, pc)
    }
}
#[cold]
#[inline(never)]
unsafe fn grow_cons(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        let i = &*pc;
        let index = e.vm.conses.len();
        e.vm.conses.push((*r.add(i.d), *r.add(i.d + 1)));
        *r.add(i.a) = Value::reference(index, 3);
        advance(e, r, pc)
    }
}
#[cold]
unsafe fn cold(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
    unsafe {
        if cold_step(e, r, pc) {
            advance(e, r, pc)
        } else {
            Value::UNBOUND
        }
    }
}
// Owned result/error/control payloads must be dropped before tail dispatch.
// Keeping them in `cold` retains one native frame per cold instruction even
// when the Lisp program's activation depth stays constant.
#[cold]
#[inline(never)]
unsafe fn cold_step(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> bool {
    unsafe {
        let outcome = (|| -> Result<()> {
            match (*pc).op {
                Op::Cell(reg) => {
                    let index = e.vm.cells.len();
                    e.vm.cells.push(*r.add(reg));
                    *r.add(reg) = Value::reference(index, 5);
                }
                Op::Global(reg, slot) => {
                    let v = if slot == e.vm.multibyte_slot {
                        if e.vm.buffer_text_p() {
                            Value::TRUE
                        } else {
                            Value::NIL
                        }
                    } else {
                        e.vm.variable_value(slot)?
                    };
                    if v == Value::UNBOUND {
                        raise(e, "void-variable", &[Value::reference(slot + 1, 7)]);
                        return Err("unbound variable".into());
                    }
                    *r.add(reg) = v;
                }
                Op::SetGlobal(slot, _) | Op::Bind(slot, _)
                    if e.vm.protected_variable_slot(slot) =>
                {
                    raise(e, "setting-constant", &[Value::reference(slot + 1, 7)]);
                    return Err("setting constant".into());
                }
                Op::SetGlobal(slot, reg) => {
                    let value = *r.add(reg);
                    if let Err(error) = e.vm.set_variable_value(slot, value) {
                        // A variable watcher's own signal, else a forwarded store fault.
                        if let Some(escape) = e.vm.propagated_escape.take() {
                            e.escape = Some(escape);
                        } else if let Some((condition, data)) = e.vm.forwarded_condition(&error, value) {
                            e.escape = Some(Escape::Signal(condition, data));
                        }
                        return Err(error);
                    }
                }
                Op::Bind(slot, reg) => {
                    let value = *r.add(reg);
                    if let Err(error) = e.vm.bind_variable(slot, value) {
                        if let Some(escape) = e.vm.propagated_escape.take() {
                            e.escape = Some(escape);
                            return Err(error);
                        }
                        if let Some((condition, data)) = e.vm.forwarded_condition(&error, value) {
                            e.escape = Some(Escape::Signal(condition, data));
                        }
                        return Err(error);
                    }
                }
                Op::Unbind(n) => {
                    if n > e.vm.dynamic.len() - e.dynamic {
                        return Err("invalid image dynamic binding depth".into());
                    }
                    if let Some(signal) = checked_unbind(e, n) {
                        e.escape = Some(signal);
                        return Ok(());
                    }
                }
                Op::Catch(dest, tag, target) => {
                    let registry = e.vm.signal_handlers.len();
                    e.vm.signal_handlers.push(SignalHandler::Catch(*r.add(tag)));
                    e.controls.push(Control {
                        kind: ControlKind::Catch(*r.add(tag)), dest, target,
                        dynamic: e.vm.dynamic.len(), registry: Some(registry),
                    });
                }
                Op::Condition(dest, conditions, target) => {
                    let registry = e.vm.signal_handlers.len();
                    e.vm.signal_handlers.push(SignalHandler::Condition(conditions));
                    e.controls.push(Control {
                        kind: ControlKind::Condition(conditions), dest, target,
                        dynamic: e.vm.dynamic.len(), registry: Some(registry),
                    });
                }
                Op::Protect(dest, target, end) => e.controls.push(Control {
                    kind: ControlKind::Protect { end },
                    dest,
                    target,
                    dynamic: e.vm.dynamic.len(),
                    registry: None,
                }),
                Op::LeaveProtect(reg) => {
                    if e.controls.len() == e.controls_base
                        || !matches!(
                            e.controls.last().map(|c| &c.kind),
                            Some(ControlKind::Protect { .. })
                        )
                    {
                        return Err("invalid image protect depth".into());
                    }
                    e.escape = Some(Escape::Normal(*r.add(reg)));
                    return Ok(());
                }
                Op::ResumeExit => {
                    if e.controls.len() == e.controls_base {
                        return Err("invalid image resume depth".into());
                    }
                    let Some(control) = e.controls.pop() else {
                        return Err("invalid image resume depth".into());
                    };
                    let ControlKind::Resume { exit, diagnostic } = control.kind else {
                        return Err("invalid image resume region".into());
                    };
                    match exit {
                        Escape::Normal(value) => {
                            *r.add(control.dest) = value;
                            return Ok(());
                        }
                        exit => {
                            e.escape = Some(exit);
                            e.escape_hooked = true;
                            e.error = diagnostic;
                        }
                    }
                }
                Op::PopHandlers(n) => {
                    let keep = e
                        .controls
                        .len()
                        .checked_sub(n)
                        .filter(|&keep| keep >= e.controls_base)
                        .ok_or("invalid image handler depth")?;
                    if e.controls[keep..].iter().any(|c| {
                        !matches!(c.kind, ControlKind::Catch(_) | ControlKind::Condition(_))
                    }) {
                        return Err("invalid image handler region".into());
                    }
                    while e.controls.len() > keep {
                        let control = e.controls.pop().unwrap();
                        release_control(e, &control);
                    }
                }
                Op::Closure(reg, function, site) => {
                    let captures = e.vm.functions[e.function].sites[site]
                        .iter()
                        .map(|&reg| *r.add(reg))
                        .collect();
                    let index = e.vm.closures.len();
                    e.vm.closures.push(Closure { function, captures, metadata: vec![], document: Value::NIL });
                    *r.add(reg) = Value::reference(index, 4);
                }
                Op::DeclareSpecial(slot) => {
                    e.vm.specials.insert(e.vm.symbol_names[slot].clone());
                }
                Op::Define(slot, reg) | Op::DefineMacro(slot, reg) => {
                    // A native `defun' is GNU's defalias (autoload queue record).
                    e.vm.autoload_note_defalias(slot);
                    e.vm.define_function(
                        slot,
                        *r.add(reg),
                        matches!((*pc).op, Op::DefineMacro(..)),
                    )?;
                }
                Op::Const(..)
                | Op::Move(..)
                | Op::ReadCell(..)
                | Op::WriteCell(..)
                | Op::Jump(..)
                | Op::Unless(..)
                | Op::Return(..)
                | Op::Call(..)
                | Op::Direct(..) | Op::ResolveHead(..) | Op::Prepared(..) => unreachable!("lowering assigns hot handlers"),
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => e.escape.is_none(),
            Err(error) => {
                e.error = Some(error);
                false
            }
        }
    }
}

#[cfg(test)]
#[path = "threaded_reader_gc_tests.rs"]
mod reader_gc_tests;

#[cfg(test)]
mod register_tests {
    use super::*;

    #[test]
    fn traversal_calls_have_specialized_handlers() {
        for name in [
            "car",
            "cdr",
            "null",
            "not",
            "assq",
            "plist-get",
            "nth",
            "setcar",
        ] {
            let mut vm = Vm::new();
            let slot = vm.slot(name);
            let argc = if matches!(name, "car" | "cdr" | "null" | "not") {
                1
            } else {
                2
            };
            let inst = lower(&vm, Op::Direct(0, slot, 0, 1, argc));
            assert!(
                !std::ptr::fn_addr_eq(inst.handler, direct as Handler),
                "{name} still uses generic dispatch"
            );
            let prepared = lower(&vm, Op::Prepared(0, 3, slot, 0, 1, argc));
            assert!(std::ptr::fn_addr_eq(prepared.prepared_handler, inst.handler),
                "HEAD_ORDER_SPECIALIZATION_LOST: {name}");
        }
    }

    #[test]
    fn entry_cache_accepts_optional_and_captured_functions_by_function_id() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun mk (x) (lambda (a b c &optional d) (+ x a))) (defvar f (mk 7)) (funcall f 1 2 3)").unwrap();
        let target = vm.slots[vm.names["f"]].value;
        let function = vm.closures[target.payload() as usize].function;
        assert!(
            vm.execution
                .entries
                .iter()
                .any(|entry| entry.function == function
                    && !entry.entry.is_null()
                    && entry.target == Value::UNBOUND),
            "captured/optional function must be cached without retaining a closure identity"
        );
    }

    #[test]
    fn cached_primitive_redefinition_and_restoration_use_authoritative_cells() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun caller () (+ 1 2 3)) (defun replacement (a b c) 42)")
            .unwrap();
        let function = vm.compile_source("(caller)").unwrap();
        assert_eq!(vm.execute_raw(function).unwrap().as_integer().unwrap(), 6);
        let plus = vm.names["+"];
        let primitive = vm.slots[plus].function;
        vm.define_function(plus, vm.slots[vm.names["replacement"]].function, false)
            .unwrap();
        assert_eq!(vm.execute_raw(function).unwrap().as_integer().unwrap(), 42);
        vm.define_function(plus, primitive, false).unwrap();
        assert_eq!(vm.execute_raw(function).unwrap().as_integer().unwrap(), 6);
    }

    #[test]
    fn primitive_calls_populate_the_entry_cache() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defvar f (lambda () 1)) (+ 1 2 3) (funcall '+ 1 2 3)")
            .unwrap();
        let target = Value::reference(Primitive::Add as usize, 6);
        assert!(
            vm.execution
                .entries
                .iter()
                .any(|entry| entry.target == target),
            "ordinary primitives must populate call-site entries"
        );
    }

    #[test]
    fn width_is_known_before_first_execution() {
        let mut vm = Vm::new();
        vm.compile_source("(lambda (a b c d e) (+ a b c d e))")
            .unwrap();
        let expected = vm.functions.iter().map(|f| f.regs).max().unwrap().max(1);
        assert_eq!(
            vm.register_width, expected,
            "definition must update register width, not the next call"
        );
    }

    #[test]
    fn reader_call_preserves_identity_roots_and_suspended_registers() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun reader-probe (x) (garbage-collect) (setcar x 42) x)")
            .unwrap();
        let function = vm.intern("reader-probe");
        let value = vm.list_value(&[Value::NIL]);
        vm.external_roots.push(value);
        vm.stack = vec![value, function];
        let registers = vm.stack.as_ptr();
        let collections = vm.gc_stats.collections;
        let result = vm.reader_call(function, &[value]).unwrap();
        assert_eq!(vm.cons_pair(result).unwrap().0.as_integer().unwrap(), 42);
        assert_eq!(vm.external_roots, vec![result]);
        assert_eq!(vm.stack, vec![result, function]);
        assert_eq!(vm.stack.as_ptr(), registers);
        assert!(vm.gc_stats.collections > collections);
    }

    #[test]
    fn reader_call_preserves_exact_signal_and_throw() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun reader-signal (x) (signal 'arith-error x)) (defun reader-throw (x) (throw 'reader-tag x))")
            .unwrap();
        let data = vm.list_value(&[Value::TRUE]);
        let signal = vm.intern("reader-signal");
        assert!(vm.reader_call(signal, &[data]).is_err());
        let (condition, actual) = vm.pending_eval_escape().unwrap();
        assert_eq!(condition, vm.intern("arith-error"));
        assert_eq!(actual, data);
        let throw = vm.intern("reader-throw");
        assert!(vm.reader_call(throw, &[data]).is_err());
        let Some(Escape::Throw(tag, actual)) = vm.propagated_escape.as_ref() else {
            panic!("callback throw was flattened");
        };
        let (tag, actual) = (*tag, *actual);
        assert_eq!(tag, vm.intern("reader-tag"));
        assert_eq!(actual, data);
    }

    #[test]
    fn retained_registers_are_zeroed_on_success_error_and_apply() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        for (index, source) in [
            "(let ((x (cons 1 2))) (funcall (lambda (a) (cons a x)) 3))",
            "(apply (lambda (a b c) (cons a (cons b c))) '(1 2 3))",
            "(funcall (lambda (x) (car 1)) (cons 1 2))",
            "(condition-case nil (apply (lambda (&rest x) (car 1)) '(1 2 3)) (error 42))",
        ]
        .into_iter()
        .enumerate()
        {
            let f = vm.compile_source(source).unwrap();
            let result = vm.execute_raw(f);
            assert_eq!(result.is_err(), index == 2, "{source}: {result:?}");
            assert!(vm.stack.is_empty(), "idle storage must not be a root set");
            assert!(!vm.execution.registers.is_empty());
            assert!(
                vm.execution.registers.iter().all(|v| *v == Value::NIL),
                "retained storage must not hold stale moving references: {source}"
            );
        }
        let f = vm.compile_source("42").unwrap();
        vm.execute_raw(f).unwrap();
        let allocation = vm.execution.registers.as_ptr();
        let capacity = vm.execution.registers.capacity();
        for _ in 0..8 {
            assert_eq!(vm.execute_raw(f).unwrap().as_integer().unwrap(), 42);
            assert_eq!(vm.execution.registers.as_ptr(), allocation);
            assert_eq!(vm.execution.registers.capacity(), capacity);
        }
    }
}
fn forwarded_fault_text(fault: crate::prim::Fault) -> String {
    crate::forwarded::fault_text(fault)
}
