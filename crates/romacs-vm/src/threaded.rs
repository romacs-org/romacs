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
use signal_handlers::{prepare_signal, release_control, speculative, DEBUGGER_HELPER};
#[path = "threaded_lend.rs"]
mod lend;
use crate::brand::{orchestrate, Gc, Plain};
use crate::gc::RootFrame;
use lend::{Nested, Orch, Pc, Write};
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

    /// Rows left to the primitive, on purpose: each does its work in the
    /// buffer, in text or by allocating, so a kernel saves little of a call.
    /// A new row needs a kernel or a place here.
    const GENERIC_BYTE_OPS: &[&str] = &[
        "current-column", "current-buffer", "widen", "end-of-line", "forward-char", "forward-line",
        "following-char", "preceding-char", "eolp", "eobp", "bolp", "bobp", "symbol-value", "set-buffer",
        "forward-word", "char-syntax", "nreverse", "skip-chars-forward", "skip-chars-backward", "set",
        "fset", "substring", "move-marker", "set-marker", "match-beginning", "match-end", "upcase",
        "downcase", "string=", "string<", "string-equal", "string-lessp", "elt", "rplaca", "rplacd",
        "buffer-substring", "delete-region", "narrow-to-region", "concat", "indent-to-column",
        "indent-to", "cdr-safe", "not",
    ];

    #[test]
    fn every_sealed_row_has_a_kernel_or_is_named_generic() {
        for (name, min, max) in crate::sealed::rows() {
            if GENERIC_BYTE_OPS.contains(&name) {
                continue;
            }
            let covered = (min..=max.min(3)).any(|argc| {
                kernel(crate::sealed::target(name, argc).expect("a row's own count"), argc).is_some()
            });
            assert!(covered, "{name} has no kernel and is not named generic");
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
        program.retire_code(&[false, true]);
        assert!(program.inline_bodies[..2].iter().all(Option::is_none));
        assert_eq!(program.free_entries, [0, 1]);
        assert_eq!(program.entries.len(), 3, "retiring keeps the sites where they are");
        assert_eq!(program.code[1][0].site, 2);
        program.close_sites();
        assert_eq!(program.entries.len(), 1);
        assert_eq!(program.inline_bodies.len(), 1);
        assert_eq!(program.code[1][0].site, 0);
        assert!(program.free_entries.is_empty());
        // Immediate-only descriptor evaluation never dereferences the argument pointer.
        assert_eq!(
            // SAFETY: the test's inline body reads no argument.
            unsafe { program.inline_bodies[0].unwrap().evaluate(std::ptr::null()) },
            Some(Value::integer(12).unwrap())
        );
        // Past half free, retiring closes the tables and keeps no free list.
        let dead = 3 * SPARE_SITES;
        program.code.insert(0, vec![instructions[0]; dead]);
        for (site, inst) in program.code[0].iter_mut().enumerate() {
            inst.site = site + 1;
        }
        program.entries.resize(dead + 1, entry);
        program.inline_bodies.resize(dead + 1, None);
        program.retire_code(&[false, true, true]);
        assert_eq!(program.entries.len(), 1);
        assert_eq!(program.free_entries.capacity(), 0);
    }

    #[test]
    fn forgotten_code_keeps_no_room_from_its_peak() {
        let mut program = Program::default();
        program.entries.reserve(SPARE_SITES * 64);
        program.inline_bodies.reserve(SPARE_SITES * 64);
        program.free_entries.reserve(SPARE_SITES * 64);
        program.forget_code();
        assert!(program.entries.capacity() <= SPARE_SITES && program.inline_bodies.capacity() <= SPARE_SITES
            && program.free_entries.capacity() <= SPARE_SITES);
    }

    fn empty_engine(vm: &mut Vm) -> Engine<'_> {
        Engine {
            vm,
            code: vec![],
            rest: RestStack::default(),
            frames: vec![],
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
            leaf_fault: None,
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
                // A test's run: no Value is kept across it.
                cold(Eng::lend(&mut e, crate::brand::Gc::unchecked()), regs.as_mut_ptr(), instructions.as_ptr());
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
        // SAFETY: a test's routing keeps no Value across it.
        assert!(orchestrate(Eng::lend(&mut e, unsafe { Gc::unchecked() }), (), route_escape, routed).is_none());
        assert_eq!(e.error.as_deref(), Some("invalid image protect depth"));
        assert_eq!(e.controls.len(), 1);
    }

    #[test]
    fn suspended_caller_regions_are_exclusive_forwarded_roots() {
        let mut vm = Vm::new();
        vm.set_gc_moving(true);
        for _ in 0..20 {
            vm.conses.push((Value::NIL, Value::NIL));
        }
        let tag = vm.list_value(&[Value::integer(17).unwrap()]);
        let payload = vm.list_value(&[Value::integer(42).unwrap()]);
        let conditions = vm.list_value(&[Value::integer(73).unwrap()]);
        let mut e = Engine {
            vm: &mut vm,
            code: vec![],
            rest: RestStack::default(),
            frames: vec![Frame {
                pc: std::ptr::null(),
                regs: std::ptr::null_mut(),
                dest: 0,
                function: 0,
                dynamic: 0,
                controls_base: 0,
                mark: NO_MARK,
            }],
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
            leaf_fault: None,
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

/// A raw handler's engine: moved into its dispatch (`crate::brand`).
pub(super) type Eng<'x, 'a> = crate::brand::Eng<'x, Engine<'a>>;
type Handler = unsafe fn(Eng<'_, '_>, *mut Value, *const Inst) -> Value;
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
    kernel: Handler,
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

// SAFETY: its `entry' points into its program's code, and only the engine
// running that program follows it; a mark's threads read the rest.
unsafe impl Sync for EntryCache {}

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
    if f.pending.is_some() || f.captures != 0 || f.rest || f.required != f.arity || f.code.len() > 8 {
        return None;
    }
    let mut operands = vec![None; f.regs];
    for (n, operand) in operands.iter_mut().take(f.arity).enumerate() {
        *operand = Some(InlineOperand::Argument(n));
    }
    let mut body = None;
    let mut result_reg = None;
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
            Op::Direct(dst, slot, _, start, argc)
            | Op::Sealed(dst, slot, start, argc) if body.is_none() => {
                let sealed = matches!(*op, Op::Sealed(..));
                let target = if sealed { sealed::target(&vm.symbol_names[slot], argc)? } else { vm.slots[slot].function };
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
                if argc != arity || !sealed && vm.macros.contains(&slot) {
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
    /// # Safety
    /// ARGS points at the call's argument registers, at least as many as the
    /// body has parameters.
    unsafe fn evaluate(&self, args: *const Value) -> Option<Value> {
        let read = |operand| match operand {
            // SAFETY: ARGS as the function's contract; N is one of the body's parameters.
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
/// The call sites a forgotten program keeps room for (`forget_code`).
const SPARE_SITES: usize = 1 << 14;
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
    /// Whether it holds no lowered code: no instruction, whose handlers are
    /// function pointers, and no entry cache, which points into the code.
    pub(super) fn holds_nothing(&self) -> bool {
        self.code.is_empty() && self.entries.is_empty()
    }
    #[cfg(test)]
    pub(super) fn code_is_empty(&self) -> bool {
        self.code.iter().all(Vec::is_empty) && self.entries.len() == self.free_entries.len()
    }
    /// Drop the lowered code an idle program caches; it is lowered again on
    /// first use. The per-site tables keep room for SPARE_SITES at most, so a
    /// peak (a nested load's millions of call sites) leaves no capacity
    /// behind, while an ordinary nested run reuses its room; `code` is
    /// bounded by the function count and keeps its own. Nothing points into
    /// it: no engine runs this program.
    pub(super) fn forget_code(&mut self) {
        self.code.clear();
        self.entries.clear();
        self.entries.shrink_to(SPARE_SITES);
        self.inline_bodies.clear();
        self.inline_bodies.shrink_to(SPARE_SITES);
        self.free_entries.clear();
        self.free_entries.shrink_to(SPARE_SITES);
    }
    pub(super) fn code_len(&self) -> usize {
        self.code.iter().map(Vec::len).sum()
    }
    #[cfg(test)]
    pub(super) fn sites(&self) -> usize {
        self.entries.len()
    }
    /// What function ID's lowered code holds, each instruction read on a copy,
    /// and the functions its entry caches enter: for verify's reference
    /// marker (gc_check.rs), apart from `trace_function'.
    pub(super) fn lowered_values(&self, id: usize, value: &mut impl FnMut(Value), function: &mut impl FnMut(usize)) {
        for inst in self.code.get(id).into_iter().flatten() {
            let mut copy = *inst;
            copy.visit_values(&mut |v| value(*v));
            let entry = self.entries[inst.site];
            value(entry.target);
            if !entry.entry.is_null() {
                function(entry.function);
            }
        }
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
    /// Drop the code of every function MARKS leaves out, its sites free for
    /// reuse: the work follows what died, not what lives. Past half free
    /// the site tables close up, so a peak is not kept and the cost of
    /// closing them amortises over the sites freed.
    pub(super) fn retire_code(&mut self, marks: &[bool]) {
        for (id, &live) in marks[..self.code.len()].iter().enumerate() {
            // Writing an empty slot would dirty a forked worker's pages.
            if !live && !self.code[id].is_empty() {
                self.release_entry(id);
            }
        }
        if self.free_entries.len() > (self.entries.len() - self.free_entries.len()).max(SPARE_SITES) {
            self.close_sites();
        }
    }
    /// Renumber the live sites densely, leaving none free.
    fn close_sites(&mut self) {
        let old = std::mem::take(&mut self.entries);
        let old_inline = std::mem::take(&mut self.inline_bodies);
        self.free_entries = Vec::new();
        for inst in self.code.iter_mut().flatten() {
            let entry = old[inst.site];
            self.inline_bodies.push(old_inline[inst.site]);
            inst.site = self.entries.len();
            self.entries.push(entry);
        }
    }
    pub(super) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit_code(&mut self.code, visit);
        for entry in &mut self.entries {
            visit(&mut entry.target);
        }
    }
}
/// A suspended engine's own state, moved here while a nested call or a
/// collection runs (`Vm::suspended`) and moved back as it resumes. The Vm owns
/// it for that long, so the collector reads and rewrites it in place, with no
/// pointer into the engine's frame and no copy. The instruction and entry
/// buffers themselves do not move, so the addresses a suspended handler holds
/// stay good; its registers stay in `Vm::stack' or `Vm::suspended_stacks'.
pub(super) struct Suspended {
    code: Vec<Vec<Inst>>,
    entries: Vec<EntryCache>,
    rest: RestStack,
    apply_function: Option<Value>,
    controls: Vec<Control>,
    escape: Option<Escape>,
    frames: Vec<Frame>,
    /// Where its registers are: `Vm::suspended_stacks[file]`, or `Vm::stack'
    /// until something sets them aside.
    file: usize,
    function: usize,
    regs: *mut Value,
}
/// An activation of a suspended engine, for what must name one: which engine,
/// whether a primitive's continuation (K_CODE) runs it, its function, and
/// where its registers start in the engine's file.
#[derive(Clone, Copy)]
pub(super) struct Activation {
    pub(super) engine: usize,
    pub(super) continuation: bool,
    pub(super) function: usize,
    pub(super) file: usize,
    pub(super) start: usize,
}
impl Suspended {
    pub(super) fn code_len(&self) -> usize {
        self.code.iter().map(Vec::len).sum()
    }
    /// The Values its code and entry caches hold, each instruction read on a
    /// copy: what a mark's threads may do while they work.
    pub(super) fn code_values(&self, value: &mut impl FnMut(Value)) {
        for inst in self.code.iter().flatten() {
            let mut copy = *inst;
            copy.visit_values(&mut |v| value(*v));
        }
        for entry in &self.entries {
            value(entry.target);
        }
    }
    /// Whether a cleanup of this engine is running: an escape's or a normal
    /// exit's, its record not yet popped (`Op::ResumeExit').
    pub(super) fn in_cleanup(&self) -> bool {
        self.controls.iter().any(Control::is_cleanup)
    }
    /// Every Value but its code's and entry caches'.
    pub(super) fn visit_state_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        self.rest.visit_values(visit);
        if let Some(function) = &mut self.apply_function { visit(function); }
        for control in &mut self.controls {
            control.visit_values(visit);
        }
        if let Some(exit) = &mut self.escape {
            exit.visit_values(visit);
        }
    }
    /// Every Value it holds, for a compaction's rewrite.
    pub(super) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit_code(&mut self.code, visit);
        for entry in &mut self.entries {
            visit(&mut entry.target);
        }
        self.visit_state_values(visit);
    }
    /// Its activations, oldest first: those its frames saved, then the running one.
    fn activations(&self, engine: usize, base: usize) -> impl Iterator<Item = Activation> + '_ {
        let start = move |regs: *mut Value| (regs as usize).wrapping_sub(base) / std::mem::size_of::<Value>();
        self.frames.iter().map(move |frame| Activation {
            engine,
            continuation: K_CODE.as_ptr_range().contains(&frame.pc),
            function: frame.function as usize,
            file: self.file,
            start: start(frame.regs),
        }).chain(std::iter::once(Activation { engine, continuation: false, function: self.function, file: self.file, start: start(self.regs) }))
    }
}
impl Vm {
    /// The activations of every suspended engine, oldest engine first: which
    /// engine and function holds which registers.
    pub(super) fn activations(&self) -> Vec<Activation> {
        self.suspended.iter().enumerate().flat_map(|(engine, record)| {
            let file = self.suspended_stacks.get(record.file).unwrap_or(&self.stack);
            record.activations(engine, file.as_ptr() as usize)
        }).collect()
    }
    /// The same as Lisp data: ((ENGINE KIND FUNCTION FILE START)...), KIND
    /// `function' or `continuation', the others integers.
    pub(crate) fn suspended_activations(&mut self) -> Value {
        let number = |n: usize| Value::integer(n as i64).expect("an index fits a fixnum");
        let (function, continuation) = (self.intern("function"), self.intern("continuation"));
        let mut list = Value::NIL;
        for a in self.activations().into_iter().rev() {
            let kind = if a.continuation { continuation } else { function };
            let row = self.list_value(&[number(a.engine), kind, number(a.function), number(a.file), number(a.start)]);
            list = self.keymap_cons(row, list);
        }
        list
    }
}
fn visit_code(code: &mut [Vec<Inst>], visit: &mut impl FnMut(&mut Value)) {
    for inst in code.iter_mut().flatten() {
        inst.visit_values(visit);
    }
}
impl Inst {
    /// Every Value it holds: the one visitor, which a read takes on a copy.
    fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.value);
        self.op.visit_values(visit);
    }
}
/// One record per call: the suspended caller's resume state, and the
/// backtrace entry the entered activation gives back when it leaves. Six
/// words (`frame_is_48_bytes_without_drop`): register and function ids are
/// narrow.
#[derive(Clone, Copy)]
struct Frame {
    pc: *const Inst,
    regs: *mut Value,
    dest: u32,
    function: u32,
    dynamic: usize,
    // Suspended caller's floor; the callee's floor is the shared stack length at entry.
    controls_base: usize,
    /// The activation's backtrace entry, or NO_MARK. A physical activation
    /// can own several real calls (an apply chain); a continuation
    /// activation (K_CODE) owns its primitive's.
    mark: usize,
}
const NO_MARK: usize = usize::MAX;
pub(super) enum Escape {
    Normal(Value),
    Throw(Value, Value),
    Signal(Value, Value),
    Error { condition: Value, object: Value, conditions: Value, target: Option<usize> },
}
romacs_snapshot::snapshot_enum!(Escape {
    Escape::Normal(f0) => (f0),
    Escape::Throw(f0, f1) => (f0, f1),
    Escape::Signal(f0, f1) => (f0, f1),
    Escape::Error { condition, object, conditions, target } => (condition, object, conditions, target),
});
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
impl Control {
    fn is_cleanup(&self) -> bool {
        matches!(self.kind, ControlKind::Resume { .. })
    }
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
/// Unbind N bindings when no variable is trapped: no watcher runs, so no
/// Lisp, and nothing can collect. The first restore that signals is consumed
/// and its escape returned, like unbind_to's pop. None, with nothing done,
/// when a trapped variable makes the unbinding a lend (`unbind_lent`).
fn unbind_now(e: &mut Engine<'_>, n: usize) -> Option<Option<Escape>> {
    if n > 0 && !e.vm.trapped.is_empty() {
        return None;
    }
    for _ in 0..n {
        if e.vm.unbind_one().is_err() {
            return Some(e.vm.propagated_escape.take());
        }
    }
    Some(None)
}
/// `unbind_now`'s lend: the unlet watchers may collect. True when a restore
/// signalled; its escape is then the pending one.
fn unbind_lent<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, n: usize) -> bool {
    lend::unbind(m, gc, n, (), |e, (), signal| signal.map(|signal| e.escape = Some(signal)).is_some())
}
/// Unbind N from orchestration: in a leaf when nothing is trapped, else lent.
/// True when a restore signalled; its escape is then the pending one.
#[inline]
fn unbind_from<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, n: usize) -> bool {
    let done = m.leaf(n, |e, n| unbind_now(e, n).map(|signal| signal.map(|signal| e.escape = Some(signal)).is_some()));
    match done {
        Some(signalled) => signalled,
        None => unbind_lent(m, gc, n),
    }
}
/// GNU signal_or_quit: call the edebug hook before unwinding, borrowing at
/// most twenty levels from the reserve, on the pending signal's CONDITION
/// and DATA. Whether its escape replaced the pending one.
#[cold]
#[inline(never)]
fn signal_hook<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, failed_depth: Option<usize>) -> bool {
    let form = m.leaf((), |e, ()| {
        let Some(&Escape::Signal(condition, data)) = e.escape.as_ref() else { return None };
        let slot = e.vm.slot("signal-hook-function");
        let hook = e.vm.variable_value(slot).ok()?;
        if hook == Value::NIL || hook == Value::UNBOUND || speculative(e, condition) { return None; }
        Some(Form::List(vec![Form::Sym("funcall".into()), quoted(e.vm, hook), quoted(e.vm, condition), quoted(e.vm, data)]))
    });
    form.is_some_and(|form| run_in_signal(m, gc, &form, 20, failed_depth).is_err())
}

/// GNU maybe_call_debugger and call_debugger past the tests
/// `debugger_wanted' makes, on the pending error's CONDITIONS and OBJECT:
/// the Lisp half tests them, then `debugger' runs under call_debugger's
/// bindings, borrowing two hundred levels. The form is the signal path's
/// own, so the debugger's frame sits on the signalling one, as GNU's C
/// leaves it. Whether its escape replaced the pending one.
#[cold]
#[inline(never)]
pub(super) fn call_debugger<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, failed_depth: Option<usize>) -> bool {
    let (test, call) = m.leaf((), |e, ()| {
        let Some(&Escape::Error { conditions, object, .. }) = e.escape.as_ref() else { unreachable!("a pending error") };
        let sym = |name: &str| Form::Sym(name.into());
        let bind = |name: &str, value: &str| Form::List(vec![sym(name), sym(value)]);
        let test = Form::List(vec![sym(DEBUGGER_HELPER), quoted(e.vm, conditions), quoted(e.vm, object)]);
        let call = Form::List(vec![sym("funcall"), sym("debugger"), Form::List(vec![sym("quote"), sym("error")]), quoted(e.vm, object)]);
        let bindings = Form::List(vec![bind("debugger-may-continue", "t"), bind("inhibit-redisplay", "nil"),
                                       bind("inhibit-debugger", "t"), bind("inhibit-changing-match-data", "nil")]);
        // GNU tests eligibility in C: the helper's calls are no stops for a
        // `debug-on-next-call' (debug-on-event's). The debugger runs after the
        // hold, so it may arm the variable again (`d').
        e.vm.profiler.held += 1;
        (test, Form::List(vec![sym("let"), bindings, call]))
    });
    let wanted = run_in_signal(m, gc.by_ref(), &test, 200, failed_depth);
    m.leaf((), |e, ()| e.vm.profiler.held -= 1);
    match wanted {
        Err(()) => true,
        Ok(false) => false,
        Ok(true) => run_in_signal(m, gc, &call, 200, failed_depth).is_err(),
    }
}

/// VALUE as a quoted, rooted form object.
fn quoted(vm: &Vm, value: Value) -> Form {
    Form::List(vec![Form::Sym("quote".into()), Form::Object(handle::FormObject::new(vm, value))])
}

/// Run FORM on the signal path, before unwinding, borrowing ROOM levels; the
/// escape it leaves replaces the pending one (Err). Its objects are rooted
/// handles, and the pending escape is engine state the suspension traces.
/// Ok(whether its value is non-nil), read at once.
fn run_in_signal<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, form: &Form, room: i64, failed_depth: Option<usize>) -> std::result::Result<bool, ()> {
    let around = m.leaf((room, failed_depth), |e, (room, failed_depth)| {
        let saved_depth = e.vm.eval_depth;
        // A refused activation has already incremented GNU's depth, even though
        // no register frame was installed for it.
        e.vm.eval_depth = failed_depth.unwrap_or(e.depth_base + e.frames.len());
        let token = e.vm.max_ensure_room(room);
        e.vm.propagated_escape = None;
        e.vm.propagated_eval_depth = None;
        e.vm.propagated_escape_hooked = false;
        (saved_depth, token)
    });
    // Compiling the call can widen the registers; the rebase follows them.
    lend::nested(m, gc, Nested::Form(form), around, |e, (saved_depth, token), result, fits| {
        let replacement = e.vm.propagated_escape.take();
        if let Some(old) = token { e.vm.restore_stack_limits(old); }
        e.vm.eval_depth = saved_depth;
        if !fits {
            e.error = Some("register storage exhausted".into());
        }
        match (result, replacement) {
            (_, Some(escape)) => {
                e.escape = Some(escape);
                Err(())
            }
            (Err(error), None) => {
                let message = e.vm.make_lisp_string(error.into_bytes(), true);
                let data = e.vm.list_value(&[message]);
                e.escape = Some(Escape::Signal(e.vm.intern("error"), data));
                Err(())
            }
            (Ok(value), None) => Ok(value != Value::NIL),
        }
    })
}
/// The escape an engine error leaves, when no escape is pending: its
/// message's condition, with the message as data for a plain `error'.
fn unrouted(e: &mut Engine<'_>, original: Option<&str>) -> Escape {
        let text = original.unwrap_or("error");
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
        let data = match original {
            Some(message) if condition == "error" => {
                let message = e.vm.make_lisp_string(message.as_bytes().to_vec(), true);
                e.vm.list_value(&[message])
            }
            _ => Value::NIL,
        };
        Escape::Signal(Value::reference(slot + 1, 7), data)
}
/// Pop the innermost control and land the pending escape there if it takes
/// it, Some(the target in this activation's code, whether a cleanup): a protect region's cleanup keeps the exit (with ORIGINAL, the
/// diagnostic) in its resume record; a catch or condition case takes the
/// payload into its register.
#[inline]
fn land(e: &mut Engine<'_>, original: &Option<String>) -> Option<(usize, bool)> {
    let control = e.controls.pop().unwrap();
    let matches = match (&control.kind, e.escape.as_ref().expect("a routed escape")) {
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
    if !matches { return None; }
    let escape = e.escape.take().unwrap();
    let cleanup = matches!(control.kind, ControlKind::Protect { .. });
    match control.kind {
        ControlKind::Protect { end } => {
            // Its cleanup runs until `Op::ResumeExit' pops the record.
            e.controls.push(Control {
                kind: ControlKind::Resume {
                    exit: escape,
                    diagnostic: original.clone(),
                },
                dest: control.dest,
                target: end,
                dynamic: control.dynamic,
                registry: None,
            });
        }
        ControlKind::Catch(_) | ControlKind::Condition(_) => {
            // Caught: a quit from C-g unwinding to here ends here,
            // and a throw's no-catch caught here retires its mark.
            e.vm.quit_unwinding = false;
            if let Escape::Error { object, .. } = &escape {
                e.vm.retire_thrown_no_catch(*object);
            }
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
            e.set_reg(control.dest, payload);
        }
        ControlKind::Resume { .. } | ControlKind::HandlerBind { .. } => unreachable!(),
    }
    Some((control.target, cleanup))
}
/// Leave this activation for its caller's (true), or end the engine with the
/// escape as its error, or as the escape it propagates to an `eval' caller.
fn leave_frame(e: &mut Engine<'_>, original: &Option<String>) -> bool {
    if let Some(frame) = e.frames.pop() {
        leave_call(e, frame.mark);
        e.rest.leave(e.frames.len() + 1);
        e.regs = frame.regs;
        e.function = frame.function as usize;
        e.dynamic = frame.dynamic;
        e.controls_base = frame.controls_base;
        return true;
    }
    let escape = e.escape.take().expect("a routed escape");
    let mut original = original.clone();
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
        return false;
    }
    e.error = Some(original.unwrap_or_else(|| match escape {
        Escape::Throw(_, _) => "no-catch".into(),
        Escape::Signal(condition, _) | Escape::Error { condition, .. } => format!("unhandled signal {}",
            e.vm.display(condition).unwrap_or_else(|_| "invalid condition".into())),
        Escape::Normal(_) => unreachable!(),
    }));
    false
}
/// A throw no catch takes becomes GNU's `no-catch' signal (true).
#[inline]
fn no_catch(e: &mut Engine<'_>) -> bool {
    let Some(Escape::Throw(tag, value)) = e.escape else { return false };
    if (e.propagate && e.depth_base <= 1)
        || e.vm.signal_handlers.iter().rev()
            .take_while(|h| !matches!(h, SignalHandler::Boundary))
            .any(|h| matches!(h, SignalHandler::Catch(wanted) if *wanted != Value::NIL && *wanted == tag))
    {
        return false;
    }
    let slot = e.vm.slot("no-catch");
    let data = e.vm.list_value(&[tag, value]);
    e.escape = Some(Escape::Signal(Value::reference(slot + 1, 7), data));
    true
}
/// Regions belong to register activations. Unwinding consumes those same frames;
/// no record copies a frame pointer, register pointer, or function identity.
/// The registers and instruction to resume at, and whether that enters a
/// cleanup, or None as the engine ends. The escape stays the pending one,
/// traced, across every restore and handler that runs Lisp on the way.
#[inline(never)]
fn route_escape<'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, (): ()) -> Option<(usize, bool)> {
    let (mut original, failed_depth, hooked) = m.leaf((), |e, ()| {
        let original = e.error.take();
        if e.escape.is_none() {
            e.escape = Some(unrouted(e, original.as_deref()));
        }
        (original, e.failed_eval_depth.take(), std::mem::take(&mut e.escape_hooked))
    });
    if prepare_signal(&mut m, gc.by_ref(), hooked, failed_depth) { original = None; }
    // Marked a throw only if no handler-bind or debugger replaced it.
    if m.leaf((), |e, ()| no_catch(e)) && !prepare_signal(&mut m, gc.by_ref(), false, None) {
        m.leaf((), |e, ()| if let Some(Escape::Error { object, .. }) = e.escape { e.vm.thrown_no_catch.push(object) });
    }
    // From here routing holds aftermath (`Vm::unwinding'; the driver
    // gives it back): the watchers its restores run, and the hooks and
    // handlers it calls while unwinding, start none. The signal-time calls
    // above come before any control is popped, so an escape from them only
    // replaces the signal, and a debugger they enter holds nothing back.
    m.leaf((), |e, ()| {
        e.vm.unwinding += 1;
        leave_native(e);
        e.apply_function = None;
        if let Some(mark) = e.apply_mark.take() { e.vm.trace_leave(mark); }
    });
    loop {
        // Unwind while each intervening handler is still live: restoring
        // a saved integer forwarding can replace the pending escape.
        while let Some(n) = m.leaf((), |e, ()| (e.controls.len() > e.controls_base).then(|| e.vm.dynamic.len() - e.controls.last().unwrap().dynamic)) {
            if unbind_from(&mut m, gc.by_ref(), n) {
                prepare_signal(&mut m, gc.by_ref(), false, None);
                original = None;
                continue;
            }
            if let Some(landing) = m.leaf(&original, |e, original| land(e, original)) {
                return Some(landing);
            }
        }
        // A normal region completion must never cross a call boundary.
        let normal = m.leaf((), |e, ()| {
            let normal = matches!(e.escape, Some(Escape::Normal(_)));
            if normal {
                e.escape = None;
                e.error = Some("invalid image protect depth".into());
            }
            normal
        });
        if normal {
            return None;
        }
        let n = m.leaf((), |e, ()| e.vm.dynamic.len() - e.dynamic);
        if unbind_from(&mut m, gc.by_ref(), n) {
            prepare_signal(&mut m, gc.by_ref(), false, None);
            original = None;
            continue;
        }
        if !m.leaf(&original, |e, original| leave_frame(e, original)) {
            return None;
        }
    }
}
/// The driver's continuation of `route_escape`: the registers and
/// instruction to resume at, and whether that enters a cleanup.
fn routed(e: Eng<'_, '_>, landing: Option<(usize, bool)>) -> Option<(*mut Value, *const Inst, bool)> {
    let (target, cleanup) = landing?;
    let code = &e.code[e.function];
    assert!(target < code.len(), "a landing inside its activation's code");
    Some((e.regs, code[target..].as_ptr(), cleanup))
}
// This is argument-stack storage, not Lisp heap objects. Its capacity is reused
// across calls and every live element is forwarded at the engine's safepoints.
#[derive(Default)]
struct RestStack {
    values: Vec<Value>,
    /// The activations that own a window of `values`, innermost last: only
    /// calls of a `&rest' function with extra arguments, keyed by their
    /// frame depth (`Engine::frames.len()` while they run).
    activations: Vec<RestActivation>,
}
#[derive(Clone, Copy)]
struct RestActivation {
    depth: usize,
    base: usize,
    pending: Option<OpaqueRest>,
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
    // Out of line and by scalars, as `leave_call`: a push inlined into a
    // dispatching handler, or a struct argument's stack temporary whose
    // address escapes, keeps the handler from tail-calling the next
    // instruction (`funcall_and_apply_tail_dispatch_has_bounded_native_stack`).
    #[inline(never)]
    fn enter(&mut self, depth: usize, reg: usize, start: usize, end: usize) {
        self.activations.push(RestActivation { depth, base: start, pending: Some(OpaqueRest { reg, start, end }) });
    }
    /// The activation at DEPTH leaves: its window goes with it.
    #[inline(never)]
    fn leave(&mut self, depth: usize) {
        if let Some(activation) = self.activations.pop_if(|a| a.depth == depth) {
            self.values.truncate(activation.base);
        }
    }
    fn pending(&self, depth: usize) -> Option<OpaqueRest> {
        self.activations.last().filter(|a| a.depth == depth).and_then(|a| a.pending)
    }
    fn materialized(&mut self, depth: usize) {
        if let Some(a) = self.activations.last_mut().filter(|a| a.depth == depth) {
            a.pending = None;
        }
    }
    fn materialize(&self, vm: &mut Vm, rest: OpaqueRest) -> Value {
        let mut tail = Value::NIL;
        // Arena pushes do not collect; collection is only at explicit strand
        // wait points. No Value copy survives a safepoint in this routine.
        for i in (rest.start..rest.end).rev() {
            tail = vm.alloc_cons(self.values[i], tail);
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
    /// A site leaf's fault, on its way to `leaf_failed'.
    leaf_fault: Option<prim::Fault>,
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
    fn in_cleanup(&self) -> bool {
        self.controls.iter().any(Control::is_cleanup)
    }
    /// `Vm::safe_runner' from this running engine, its own cleanups included.
    fn safe_runner(&mut self) -> Option<Value> {
        if self.in_cleanup() { None } else { self.vm.safe_runner(1) }
    }
    #[cfg(test)]
    fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit_code(&mut self.code, visit);
        for entry in &mut self.entries {
            visit(&mut entry.target);
        }
        self.rest.visit_values(visit);
        if let Some(function) = &mut self.apply_function { visit(function); }
        for control in &mut self.controls {
            control.visit_values(visit);
        }
        if let Some(exit) = &mut self.escape {
            exit.visit_values(visit);
        }
    }
}
#[cfg(test)]
pub(super) fn assert_opaque_rest_compaction() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
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
            activations: vec![RestActivation { depth: 0, base: 0, pending: Some(desc) }],
        },
        stack_used: 0,
        apply_scratch: Vec::new(),
        frames: vec![],
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
            leaf_fault: None,
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

/// Arguments a native call into Lisp passes in registers; more go to
/// `apply' as one list (`Vm::reader_trampoline`).
const TRAMPOLINE_ARITY: usize = 8;

impl Vm {
    /// Call FUNCTION with ARGS from native code, the caller's register stack
    /// set aside (traced, its allocation unmoved for suspended engines' raw
    /// pointers). FUNCTION and ARGS are rooted in the trampoline's registers;
    /// the caller keeps nothing else across the call unless it is rooted.
    pub(crate) fn reader_call(&mut self, function: Value, args: &[Value]) -> Result<Value> {
        // Compiling a trampoline allocates nothing that can collect.
        let trampoline = self.reader_trampoline(args.len())?;
        let registers: Vec<Value> = if args.len() <= TRAMPOLINE_ARITY {
            std::iter::once(function).chain(args.iter().copied()).collect()
        } else {
            vec![function, self.list_value(args)]
        };
        // A leaf's Lisp comes here, by pending variables, `safe_call' and
        // watchers: it sees the leaf's frame, whichever way it came.
        self.with_site_leaf(|vm| {
            vm.suspend_stack();
            vm.propagated_escape = None;
            vm.propagated_escape_hooked = false;
            vm.propagated_eval_depth = None;
            vm.propagate_next = true;
            let result = execute_args(vm, trampoline, &registers);
            vm.propagate_next = false;
            vm.resume_stack();
            vm.end_nested_execution();
            result
        })
    }
    /// BODY, with the activation recorded of any leaf a site is running with
    /// none: what BODY runs or publishes sees that frame.
    pub(crate) fn with_site_leaf<T>(&mut self, body: impl FnOnce(&mut Self) -> T) -> T {
        let site = std::mem::take(&mut self.site_leaf);
        let mark = self.record_site_leaf(site);
        let result = body(self);
        if let Some(mark) = mark {
            self.trace_leave(mark);
        }
        self.site_leaf = site;
        result
    }
    /// The activation `trace_call' would have made for the leaf at SITE.
    fn record_site_leaf(&mut self, site: (usize, usize)) -> Option<usize> {
        let (pc, r) = (site.0 as *const Inst, site.1 as *mut Value);
        if pc.is_null() {
            return None;
        }
        // SAFETY: `leaf_call' set SITE and is still running: the instruction
        // and its registers are live, and nothing has widened them.
        unsafe {
            let (function, resolved, start, argc) = match (*pc).op {
                Op::Direct(_, slot, _, start, argc) => (Value::reference(slot + 1, 7), self.slots[slot].function, start, argc),
                Op::Sealed(_, slot, start, argc) => (Value::reference(slot + 1, 7), (*pc).value, start, argc),
                Op::Call(_, function, start, argc) => {
                    let function = *r.add(function);
                    (function, self.backtrace_resolve(function).unwrap_or(function), start, argc)
                }
                _ => return None,
            };
            let args = backtrace::Arguments::Registers {
                stack: self.suspended_stacks.len(),
                offset: r.add(start).offset_from(self.stack.as_ptr()) as usize,
                count: argc,
            };
            Some(self.backtrace.enter(function, resolved, args, self.dynamic.len()))
        }
    }
    /// The permanent function a native call of N arguments enters:
    /// `(lambda (f a1 .. aN) (funcall f a1 .. aN))`, or for more than
    /// TRAMPOLINE_ARITY `(lambda (f args) (apply f args))`. Its own `Call`
    /// does what any Lisp call does (captures, arity, `&rest`, autoload,
    /// escapes), so no native call compiles a request form. Lexical and
    /// eager whatever is being compiled or evaluated now, its parameters
    /// uninterned: the callee sees no binding of its own.
    fn reader_trampoline(&mut self, n: usize) -> Result<usize> {
        let index = n.min(TRAMPOLINE_ARITY + 1);
        if let Some(&Some(id)) = self.reader_trampolines.get(index) {
            return Ok(id);
        }
        let spread = index > TRAMPOLINE_ARITY;
        let parameter = || Form::Generated(crate::expand::generated_id());
        let params: Vec<Form> = (0..if spread { 2 } else { index + 1 }).map(|_| parameter()).collect();
        let head = if spread { "apply" } else { "funcall" };
        let body = Form::List(std::iter::once(Form::Sym(head.into())).chain(params.iter().cloned()).collect());
        let scope = std::mem::replace(&mut self.dynamic_scope, false);
        let regime = std::mem::replace(&mut self.lazy_regime, false);
        let compiled = self.compile_permanent(|vm| vm.function(&params, &[body], &NameMap::default(), &NameSet::default()));
        self.lazy_regime = regime;
        self.dynamic_scope = scope;
        let (id, _) = compiled?;
        if self.reader_trampolines.len() <= index {
            self.reader_trampolines.resize(index + 1, None);
        }
        self.reader_trampolines[index] = Some(id);
        Ok(id)
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
    // SAFETY: an unconverted caller's entry: its file's census rows say what
    // it keeps across the run.
    let mut gc: crate::brand::Gc<'_, '_> = unsafe { crate::brand::Gc::unchecked() };
    vm.execution_fatal = false;
    vm.require_live_function(function)?;
    if vm.functions[function].lazy.is_some() {
        // Nested macro calls get their own engines and must not consume this
        // execution's propagation request. The open builder holds collection.
        let propagate = std::mem::take(&mut vm.propagate_next);
        let forced = vm.force_lazy(function);
        vm.propagate_next = propagate;
        forced?;
    }
    if let Err(error) = vm.materialize_image_function(function) {
        vm.execution_fatal = true;
        return Err(error);
    }
    debug_assert!(vm.stack.is_empty(), "a nested run's caller set its registers aside");
    let mut program = std::mem::take(&mut vm.execution);
    if program.registers.is_empty() && let Some(spare) = vm.spare_programs.pop() {
        program = spare;
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
        .and_then(|n| n.checked_add(vm.spread_reserve))
        .ok_or("register storage overflow")?;
    vm.stack = std::mem::take(&mut program.registers);
    // They come back all NIL, however far the last run widened them: cut to
    // SIZE, every run past MIN_FRAMES would fill them again.
    if vm.stack.len() < size {
        vm.stack.resize(size, Value::NIL);
    }
    vm.stack[..args.len()].copy_from_slice(args);
    let regs = vm.stack.as_mut_ptr();
    let mark = vm.dynamic.len();
    let handler_mark = vm.signal_handlers.len();
    let backtrace_mark = vm.backtrace.frames.len();
    let vm_propagate = std::mem::take(&mut vm.propagate_next);
    let mut engine = Engine {
        stack_used: vm.functions[function].regs.max(args.len()),
        apply_scratch: Vec::new(),
        depth_base: vm.eval_depth,
        vm,
        code: program.code,
        rest: program.rest,
        frames: Vec::with_capacity(512),
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
            leaf_fault: None,
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
    // This engine holds collection until it suspends; a writer's rooted
    // watcher notification does not extend into it.
    engine.vm.running_engines += 1;
    let rooted_write = std::mem::take(&mut engine.vm.rooted_write);
    if engine.vm.stress_due() && engine.vm.may_collect(1) {
        orchestrate(Eng::lend(&mut engine, gc.by_ref()), (), stress_collect, done);
    }
    // GNU's eval_sub and funcall run maybe_quit before the body: a quit
    // already due ends the entry before its first instruction.
    let quit = if crate::host::safepoint_pending() || engine.vm.quit_due() {
        engine.budget = 0;
        interrupt_quit(&mut engine)
    } else {
        None
    };
    // SAFETY: storage and lowered code satisfy the module invariants above.
    let mut value = match quit {
        Some(value) => value,
        // SAFETY: REGS is the new engine's first frame, and a null PC enters its function's code (`next`).
        None => unsafe { next(Eng::lend(&mut engine, gc.by_ref()), regs, std::ptr::null()) },
    };
    // Unoptimized Rust does not perform sibling-call elimination. Drive the
    // exact same handlers one at a time in dev builds, keeping debug safe.
    loop {
        #[cfg(debug_assertions)]
        while let Some((regs, pc)) = engine.resume.take() {
            // SAFETY: the resume record holds the frame and instruction a handler left (`next`).
            value = unsafe { ((*pc).handler)(Eng::lend(&mut engine, gc.by_ref()), regs, pc) };
        }
        if engine.fatal_image_failure || (engine.error.is_none() && engine.escape.is_none()) { break; }
        // Routing holds aftermath once it starts unwinding (`route_escape'
        // takes the hold, this gives it back).
        let routed = orchestrate(Eng::lend(&mut engine, gc.by_ref()), (), route_escape, routed);
        engine.vm.unwinding -= 1;
        let Some((regs, pc, cleanup)) = routed else { break };
        // Region exits are strand wait points too: suspended payloads are
        // activation-owned roots before any cleanup instruction can allocate.
        // A handler landing is an ordinary poll; a cleanup's entry only collects.
        let e = Eng::lend(&mut engine, gc.by_ref());
        // SAFETY: routing returned the frame its escape lands in and the instruction there.
        value = unsafe { if cleanup { cleanup_entry(e, regs, pc) } else { poll_resume(e, regs, pc) } };
    }
    // A normal return has already unbound through return_unbind. With a
    // variable trapped, the restores' watchers may collect: VALUE is rooted.
    let n = engine.vm.dynamic.len() - mark;
    if n > 0 && !engine.vm.trapped.is_empty() {
        let frame = engine.vm.root_frame(&[value]);
        orchestrate(Eng::lend(&mut engine, gc.by_ref()), n, unwind_lent, done);
        [value] = engine.vm.release_frame(frame);
    } else {
        engine.vm.unwind(n);
    }
    engine.vm.signal_handlers.truncate(handler_mark);
    engine.vm.trace_leave(backtrace_mark);
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
    engine.vm.rooted_write = rooted_write;
    // Aftermath it left pending is polled for soon by the engine it returns to.
    engine.vm.collect_poll |= engine.vm.aftermath_pending();
    engine.vm.running_engines -= 1;
    match engine.error {
        Some(error) => Err(error),
        None => Ok(value),
    }
}
/// A stress collection at an engine's entry.
#[inline(never)]
fn stress_collect<'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, (): ()) {
    lend::collect(&mut m, gc.by_ref(), false);
}
/// The driver's continuation of a lend that leaves nothing to do.
fn done(_: Eng<'_, '_>, (): ()) {}
/// The engine's exit unwinding, its unlet watchers free to collect.
#[inline(never)]
fn unwind_lent<'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, n: usize) {
    lend::unwind(&mut m, gc.by_ref(), n);
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
                        Op::Call(_, _, start, count) | Op::Direct(_, _, _, start, count)
                        | Op::Sealed(_, _, start, count) => {
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
        // Branches hold their distance, so they need no code table.
        for (at, inst) in code[id].iter_mut().enumerate() {
            if let Op::Jump(to) | Op::Unless(_, to) = inst.op {
                inst.c = (to as isize - at as isize) as usize;
            }
        }
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
        kernel: direct,
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
        Op::Sealed(a, b, d, e) => {
            i.a = a;
            i.b = b;
            i.d = d;
            i.e = e;
            i.value = sealed::target(&vm.symbol_names[b], e).expect("sealed call of a byte-op primitive");
            i.handler = kernel(i.value, e).unwrap_or(sealed_call);
        }
        Op::Global(a, b) => {
            // Two plain-looking variables whose value is the buffer's.
            i.handler = if b == vm.multibyte_slot || b == vm.truncate_lines_slot { cold } else { global };
            i.a = a;
            i.b = b;
        }
        Op::Cell(a) => {
            i.handler = cell;
            i.a = a;
        }
        Op::Closure(a, b, c) => {
            i.handler = closure;
            i.a = a;
            i.b = b;
            i.c = c;
        }
        Op::SetGlobal(a, b) => {
            i.handler = assign_global;
            i.a = a;
            i.b = b;
        }
        Op::Bind(a, b) => {
            i.handler = bind;
            i.a = a;
            i.b = b;
        }
        Op::Unbind(a) => {
            i.handler = unbind;
            i.a = a;
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
        Op::BindLate(a, b) => {
            i.handler = bind_late;
            i.a = a;
            i.b = b;
        }
        Op::UnbindLate(a) => {
            i.handler = unbind_late;
            i.a = a;
        }
        Op::ReadLate(a, b, c) => {
            i.handler = read_late;
            i.a = a;
            i.b = b;
            i.c = c;
        }
        Op::LateForwarded(a, b) => {
            i.handler = late_forwarded;
            i.a = a;
            i.b = b;
        }
        Op::WriteLate(a, b, c) => {
            i.handler = write_late;
            i.a = a;
            i.b = b;
            i.c = c;
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
            // A kernel serves the primitive the cell holds now; `direct_kernel`
            // runs it only while the site cache still holds that primitive.
            let target = vm.slots[b].function;
            if let Some(k) = kernel(target, e) {
                i.handler = direct_kernel;
                i.value = target;
                i.kernel = k;
            }
        }
        _ => {}
    }
    i
}
/// The specialized handler for a call of primitive TARGET with ARGC
/// arguments, if one exists. A kernel computes the common case in place and
/// calls TARGET itself (`sealed_call`) for the rest; it never reads the
/// function cell, so its caller must know the cell question is settled.
fn kernel(target: Value, argc: usize) -> Option<Handler> {
    if let Some(d) = prim::generated(target) {
        return match (d.name, argc) {
            ("insert", 1) => Some(insert_string),
            ("listp", 1) => Some(listp),
            ("numberp", 1) => Some(numberp),
            ("%", 2) => Some(remainder),
            ("cdr-safe", 1) => Some(cdr_safe),
            ("nconc", 2) => Some(nconc2),
            ("plist-get", 2) => Some(traversal::plist_get),
            ("symbol-function", 1) => Some(symbol_function),
            (">", 2) => Some(greater),
            ("<=", 2) => Some(less_equal),
            (">=", 2) => Some(greater_equal),
            ("nthcdr", 2) => Some(nthcdr),
            _ => None,
        };
    }
    Some(match (prim::legacy(target)?, argc) {
        (Primitive::Car, 1) => traversal::car,
        (Primitive::Cdr, 1) => traversal::cdr,
        (Primitive::Null, 1) => traversal::null,
        (Primitive::Eq, 2) => traversal::eq,
        (Primitive::Memq, 2) => traversal::memq,
        (Primitive::Assq, 2) => traversal::assq,
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
        (Primitive::Div, 2) => divide,
        (Primitive::Max, 2) => max,
        (Primitive::Min, 2) => min,
        (Primitive::Aset, 3) => aset,
        (Primitive::DeepEqual, 2) => deep_equal,
        (Primitive::Member, 2) => member,
        (Primitive::Get, 2) => get,
        (Primitive::List, _) => list,
        (Primitive::Consp, 1) => consp,
        (Primitive::Symbolp, 1) => symbolp,
        (Primitive::Stringp, 1) => stringp,
        (Primitive::Integerp, 1) => integerp,
        (Primitive::CarSafe, 1) => car_safe,
        (Primitive::Aref, 2) => sequence_read,
        (Primitive::Length, 1) => length,
        (Primitive::CharAfter, 0 | 1)
        | (Primitive::Point | Primitive::PointMin | Primitive::PointMax | Primitive::BufferSize, 0)
        | (Primitive::GotoChar, 1) => buffer_read,
        _ => return None,
    })
}
// Keep observation scratch out of dispatch handlers. Inlining this register
// walk into next gives handlers address-taken locals that prevent sibling calls
// in optimized builds, retaining native frames even in non-rest programs.
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn observe_rest(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) {
    // A pending binding cannot be copied into an ordinary Value. Materialize
    // before any instruction that can read it, including implicit captures and
    // contiguous call windows. Conservative destination matches are intentional.
    if let Some(rest) = e.rest.pending(e.frames.len()) {
        // SAFETY: R and PC as the function's contract (`# Safety`).
        let mut op = unsafe { (*pc).op };
        // These writes cannot fail or transfer control before replacing the
        // binding. Its previous tail is genuinely dead, not observed.
        if matches!(op, Op::Const(dst, _) if dst == rest.reg)
            || matches!(op, Op::Move(dst, src) if dst == rest.reg && src != rest.reg)
        {
            e.rest.materialized(e.frames.len());
            return;
        }
        let mut observed = false;
        op.map_registers(&mut |reg| {
            observed |= reg == rest.reg;
            reg
        });
        match op {
            Op::Call(_, _, start, count) | Op::Direct(_, _, _, start, count)
            | Op::Sealed(_, _, start, count) => {
                observed |= (start..start + count).contains(&rest.reg);
            }
            Op::Closure(_, _, site) => {
                observed |= e.vm.functions[e.function].sites[site].contains(&rest.reg);
            }
            _ => {}
        }
        if observed {
            let value = e.rest.materialize(e.vm, rest);
            // SAFETY: R and PC as the function's contract (`# Safety`).
            unsafe {
                *r.add(rest.reg) = value;
            }
            e.rest.materialized(e.frames.len());
        }
    }
}
// Observation may allocate, but cannot collect or reenter dispatch. The rest
// side stack and register window remain traced by Engine::visit_values.
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn observe_then(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        observe_rest(&mut e, r, pc);
        ((*pc).observed_handler)(e, r, pc)
    }
}
#[inline(always)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn next(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // A null pc is used only at entry, never stored in an instruction/frame.
    let pc = if pc.is_null() {
        e.code[e.function].as_ptr()
    } else {
        pc
    };
    #[cfg(debug_assertions)]
    {
        let mut e = e;
        e.resume = Some((r, pc));
        Value::UNBOUND
    }
    #[cfg(not(debug_assertions))]
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        // No per-instruction stack probe: it tripled straight-line dispatch
        // cost. Native stack grows per Lisp activation (T2 callbacks, cold
        // non-tail handlers), so `depth_room` checks remaining bytes there.
        ((*pc).handler)(e, r, pc)
    }
}
#[inline(always)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn advance(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
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
    let arity = e.vm.alloc_cons(min, max);
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
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn poll_resume(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    e.budget = 10_000;
    #[cfg(test)]
    POLLS.with(|polls| polls.set(polls.get() + 1));
    // R193: the dispatch below must be a sibling call or every poll keeps a
    // frame: gdb on the gate-built (non-LTO) romacs-gnu-ert adapter showed
    // 74,600 poll_resume frames and SIGABRT under `(while t)` when the
    // collection's Vec lived in this frame. Collection is therefore out of
    // line (`polled`, its every local plain), and the engine's native floor
    // still refuses recoverably should a build keep the frame anyway.
    if eval_depth::stack_pointer() < e.native_floor {
        return native_stack_failure(&mut e, "native stack exhausted during Lisp evaluation");
    }
    // Before the debugger and collection: an inspection changes nothing and
    // answers from any engine, nested or not.
    if crate::host::inspect_pending() { e.vm.inspect_capture(); }
    // SAFETY: the function's contract: PC is the instruction this poll
    // resumes, in this engine's code, and R its frame.
    unsafe {
        if e.vm.profiler.debug_call {
            return debug_on_call(e, r, pc);
        }
        debug_assert_eq!(r, e.regs, "a poll runs in the current frame");
        // Due but held (a nested engine): the request stays for the engine
        // that may collect.
        let collect = if e.vm.collection_due() { e.vm.may_collect(1) } else {
            e.vm.collect_poll = false;
            false
        };
        orchestrate(e, (Pc::new(pc), collect), polled, continue_at)
    }
}
/// `poll_resume`'s collection, a collection's aftermath and
/// the quit test, then the instruction it resumes.
#[inline(never)]
fn polled<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, (pc, collect): (Pc<'x>, bool)) -> Continue<'x> {
    if collect {
        lend::collect(&mut m, gc.by_ref(), false);
    }
    // GNU's garbage_collect ends with its finalizers and `post-gc-hook':
    // here, whatever collection left them, where the gate allows. The cheap
    // test first: most polls find neither, and a handler landing is one.
    let runner = m.leaf((), |e, ()| {
        if !e.vm.aftermath_flagged() { return None; }
        let runner = e.safe_runner();
        plain_runner(e.vm, runner)
    });
    if let Some(runner) = runner && !aftermath(&mut m, gc.by_ref(), runner) {
        return Continue::Stop;
    }
    // GNU's maybe_quit: a host's C-g (quit.rs), a debug event, or an
    // isolate's interrupt (SIGUSR1, `isolate.rs`) is `quit` at this
    // safepoint unless `inhibit-quit' is bound.
    let quit = m.leaf((), |e, ()| (crate::host::safepoint_pending() || e.vm.quit_poll_due()) && interrupt_quit(e).is_some());
    if quit { Continue::Stop } else { Continue::At(pc) }
}
/// Entering a cleanup, its exit kept in a resume record: a collection due
/// runs here, Rust only, the exit traced, as at any wait point. Nothing that
/// can raise runs (a quit, the debugger, a collection's aftermath): an escape
/// now would meet that record on top (`route_escape') and drop the cleanup
/// unrun. The next ordinary poll takes them.
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn cleanup_entry(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    let collect = e.vm.collection_due() && e.vm.may_collect(1);
    debug_assert!(!collect || r == e.regs, "a cleanup runs in the current frame");
    // SAFETY: the function's contract: PC is the cleanup's first
    // instruction, in this engine's code, and R its frame.
    unsafe {
        if collect {
            return orchestrate(e, Pc::new(pc), cleanup_collected, continue_at);
        }
        next(e, r, pc)
    }
}
#[inline(never)]
fn cleanup_collected<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, pc: Pc<'x>) -> Continue<'x> {
    lend::collect(&mut m, gc.by_ref(), false);
    Continue::At(pc)
}
/// GNU maybe_quit at a safepoint: an isolate's pending interrupt as a
/// `quit` signal, else the host's C-g or `quit-flag' (`Vm::maybe_quit').
/// Either is held while `inhibit-quit' is non-nil, for a later safepoint.
#[cold]
#[inline(never)]
fn interrupt_quit(e: &mut Engine<'_>) -> Option<Value> {
    if crate::host::interrupt_pending() && isolate_quit(e) {
        // A signal, not an engine failure: `condition-case' catches it.
        e.vm.quit_unwinding = true;
        return Some(raise(e, "quit", &[]));
    }
    let escape = e.vm.maybe_quit()?;
    e.escape = Some(escape);
    Some(Value::UNBOUND)
}
/// Whether an isolate's interrupt or termination quits here.
fn isolate_quit(e: &mut Engine<'_>) -> bool {
    // A termination quits once, whatever `inhibit-quit' says: the worker's
    // loop then ends (`romacs--isolate-terminated-p').
    if crate::host::take_terminate_quit() {
        return true;
    }
    let slot = e.vm.slot("inhibit-quit");
    if e.vm.variable_value(slot).is_ok_and(|v| v != Value::NIL && v != Value::UNBOUND) {
        return false;
    }
    // Held, not taken, while a run of aftermath masks the job: taken now it
    // would find no job to quit and be lost.
    if e.vm.aftermath_runs > 0 { return false; }
    crate::host::take_interrupt() && e.vm.isolate_interrupt_targets_running()
}
// Like GNU bytecode.c:638-646, only a plain, bound symbol takes the slot
// read. Inspect the live set-only redirect on every entry: localization need
// not publish a function-world epoch, and can happen inside a cached callee.
// No Value or slot pointer is retained across a safepoint or allocation.
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn global(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        if e.vm.variable_redirects.get(i.b) != Some(&Some(VariableRedirect::Plain)) {
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
// Late bindings (Op::BindLate): the common case, a binding made lexically,
// stays on the hot path; a dynamic one goes cold.
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn bind_late(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        if e.vm.slot_is_special_now((*pc).a) { return cold(e, r, pc); }
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn unbind_late(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        if late_get(&e, r, (*pc).a) == Value::UNBOUND { return cold(e, r, pc); }
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn read_late(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let v = late_get(&e, r, (*pc).b);
        if v == Value::UNBOUND { return cold(e, r, pc); }
        *r.add((*pc).a) = v;
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn late_forwarded(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        *r.add((*pc).a) = if late_get(&e, r, (*pc).b) == Value::UNBOUND { Value::TRUE } else { Value::NIL };
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn write_late(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        if late_get(&e, r, (*pc).a) == Value::UNBOUND { return cold(e, r, pc); }
        late_set(&mut e, r, (*pc).a, *r.add((*pc).b));
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn constant(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        *r.add((*pc).a) = (*pc).value;
        advance(e, r, pc)
    }
}
#[cfg_attr(
    all(target_os = "linux", target_arch = "x86_64"),
    // SAFETY: a section of this binary's own text, laid out by the
    // `global_asm!` above; it holds only this handler's code.
    unsafe(link_section = ".text.vm2.mov")
)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn mov(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        *r.add((*pc).a) = *r.add((*pc).b);
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn read_cell(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let v = *r.add((*pc).b);
        if v.tag() != 5 {
            return fail(&mut e, "cell required");
        }
        *r.add((*pc).a) = e.vm.read_capture_cell(v.payload() as usize);
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn write_cell(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let v = *r.add((*pc).a);
        if v.tag() != 5 {
            return fail(&mut e, "cell required");
        }
        e.vm.write_capture_cell(v.payload() as usize, *r.add((*pc).b));
        advance(e, r, pc)
    }
}
#[cfg_attr(
    all(target_os = "linux", target_arch = "x86_64"),
    // SAFETY: a section of this binary's own text, laid out by the
    // `global_asm!` above; it holds only this handler's code.
    unsafe(link_section = ".text.vm2.jump")
)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn jump(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let to = pc.offset((*pc).c.cast_signed());
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
    // SAFETY: a section of this binary's own text, laid out by the
    // `global_asm!` above; it holds only this handler's code.
    unsafe(link_section = ".text.vm2.unless")
)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn unless(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        if *r.add((*pc).a) == Value::NIL {
            next(e, r, pc.offset((*pc).c.cast_signed()))
        } else {
            advance(e, r, pc)
        }
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn ret(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let en: &mut Engine<'_> = &mut e;
        let value = *r.add((*pc).a);
        if en.vm.dynamic.len() != en.dynamic {
            return return_unbind(e, r, pc);
        }
        if en.controls.len() != en.controls_base {
            return fail(en, "invalid image return through control region");
        }
        // The return's one hook test, in place of `trace_leave''s.
        if en.vm.profiler.active
            && let Some(frame) = en.frames.last()
            && frame.mark != NO_MARK
        {
            if en.vm.profiler.exit_flags && en.vm.backtrace.exit_flagged_from(frame.mark) {
                let mark = frame.mark;
                return debug_on_exit(e, r, pc, mark);
            }
            en.vm.hooked_leave(frame.mark);
        }
        if let Some(frame) = en.frames.pop() {
            leave_return(en, frame.mark);
            en.rest.leave(en.frames.len() + 1);
            *frame.regs.add(frame.dest as usize) = value;
            en.function = frame.function as usize;
            en.dynamic = frame.dynamic;
            en.controls_base = frame.controls_base;
            en.regs = frame.regs;
            next(e, frame.regs, frame.pc)
        } else {
            value
        }
    }
}
#[cold]
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn return_unbind(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    let n = e.vm.dynamic.len() - e.dynamic;
    let now = unbind_now(&mut e, n);
    debug_assert!(now.is_some() || r == e.regs, "a return runs in the current frame");
    // SAFETY: the function's contract: PC is the return's instruction, in
    // this engine's code, and R its frame.
    unsafe {
        match now {
            Some(Some(signal)) => {
                e.escape = Some(signal);
                Value::UNBOUND
            }
            Some(None) => ret(e, r, pc),
            None => orchestrate(e, (Pc::new(pc), n), return_lent, continue_at),
        }
    }
}
/// A return's unbinding with unlet watchers, which may collect; the return
/// value is read from its register after.
#[inline(never)]
fn return_lent<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, (pc, n): (Pc<'x>, usize)) -> Continue<'x> {
    if unbind_lent(&mut m, gc.by_ref(), n) { Continue::Stop } else { Continue::Return(pc) }
}
/// A named call whose cell held a primitive with a kernel when lowered: the
/// kernel while the site cache still holds that primitive (the cache is
/// filled by `direct_generic`, which also serves the first call and any
/// redefinition), as GNU's Bcall reads the cell after the arguments.
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn direct_kernel(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let cache = e.vm.caches.get_unchecked(i.c);
        if cache.version == e.vm.slots.get_unchecked(i.b).version && cache.target == i.value {
            (i.kernel)(e, r, pc)
        } else {
            direct_generic(e, r, pc)
        }
    }
}
/// The general call of the primitive an instruction was lowered for: the
/// `Sealed` handler without a kernel, and every kernel's fallback.
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn sealed_call(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe { invoke(e, r, pc, (*pc).value, (*pc).d, (*pc).e) }
}
/// Check the prospective activation, independently of register storage.
/// The common case (limit >= 100, depth and native stack within bounds) is
/// two compares; everything else, including GNU's clamp, takes the cold path.
#[inline(always)]
fn depth_room(e: &mut Engine<'_>) -> bool {
    let attempted = e.depth_base + e.frames.len() + 1;
    // A buffer-local limit makes the current buffer's binding the one read.
    let limit = if e.vm.forwarded.eval_limit_symbol == usize::MAX { e.vm.forwarded.eval_limit() } else { e.vm.eval_limit() };
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
/// # Safety
/// R is the running engine's current frame (`Engine::regs`), holding the
/// registers the caller names.
unsafe fn frame_room(e: &mut Engine<'_>, r: *mut Value) -> Option<*mut Value> {
    let needed = (e.frames.len() + 3).checked_mul(e.vm.register_width)?;
    if needed <= e.vm.stack.len().checked_sub(e.vm.spread_reserve)? {
        Some(r)
    } else {
        // SAFETY: R as the function's contract (`# Safety`).
        unsafe { widen_registers(e, r, e.vm.register_width) }
    }
}
// Keep metadata allocation and cleanup out of threaded callers so optimized
// non-LTO builds can tail-dispatch after the operation completes.
#[inline(never)]
fn leave_native(e: &mut Engine<'_>) {
    if let Some(mark) = e.native_mark.take() { e.vm.trace_leave(mark); }
    // Native code made big payloads: poll, and collect if due, soon. The
    // request stays until a poll settles it; an engine that may not collect
    // (nested, or held) leaves it to the one it returns to, polling at its
    // own pace meanwhile.
    if e.vm.collect_poll && e.vm.may_collect(1) { e.budget = 0; }
}
#[inline(never)]
fn leave_call(e: &mut Engine<'_>, mark: usize) {
    if mark != NO_MARK { e.vm.trace_leave(mark); }
}
/// A normal return's leave: `ret' has run the hooks.
#[inline(never)]
fn leave_return(e: &mut Engine<'_>, mark: usize) {
    if mark != NO_MARK { e.vm.backtrace.leave(mark); }
}
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn trace_call<const LISP: bool>(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst,
    target: Value, start: usize, argc: usize) -> usize {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let function = if std::ptr::eq(pc, &K_CODE[0]) { *r.add(K_F) } else {
            match (*pc).op {
                Op::Direct(_, slot, ..) | Op::Sealed(_, slot, ..) => Value::reference(slot + 1, 7),
                Op::Call(_, function, ..) => *r.add(function),
                _ => target,
            }
        };
        // The spreader carries the final original function through nested
        // apply calls. Reading the outer call's first argument loses aliases
        // and can mislabel a real failing callee as apply itself.
        let spread = e.apply_function.take();
        let function = spread.unwrap_or(function);
        let resolved = if target == Value::UNBOUND { e.vm.backtrace_resolve(function).unwrap_or(target) } else { target };
        // GNU records a pointer to the caller's pushed arguments; so does this,
        // as an offset into the register stack. `apply`'s spread window is
        // the exception: a nested apply reuses it, so those are copied.
        let args = if spread.is_some() {
            e.vm.backtrace.copied(std::slice::from_raw_parts(r.add(start), argc))
        } else {
            backtrace::Arguments::Registers {
                stack: e.vm.suspended_stacks.len(),
                offset: r.add(start).offset_from(e.vm.stack.as_ptr()) as usize,
                count: argc,
            }
        };
        // One test for every call boundary hook; the hooked path is cold.
        if e.vm.profiler.active {
            return hooked_call::<LISP>(e, function, resolved, args);
        }
        e.vm.backtrace.enter(function, resolved, args, e.vm.dynamic.len())
    }
}
/// A call's backtrace entry while a hook is armed: the profilers sample the
/// stack it is entered from, and a Lisp call may stop for
/// `debug-on-next-call'.
#[cold]
#[inline(never)]
fn hooked_call<const LISP: bool>(e: &mut Engine<'_>, function: Value, resolved: Value, args: backtrace::Arguments) -> usize {
    e.vm.profile_boundary();
    let mark = e.vm.backtrace.enter(function, resolved, args, e.vm.dynamic.len());
    if LISP { arm_debug_on_call(e, mark); }
    mark
}
/// GNU do_debug_on_call for the Lisp call MARK enters, when
/// `debug-on-next-call' is set: clear it, flag the call debug-on-exit, and
/// stop at the call's first poll, its frame in place (`debug_on_call').
/// Primitives are no stops (DIVERGENCES DBG-NEXT-CALL); the variable stays
/// set through them.
#[cold]
#[inline(never)]
fn arm_debug_on_call(e: &mut Engine<'_>, mark: usize) {
    if !e.vm.profiler.next_call || e.vm.profiler.held != 0 { return; }
    e.vm.forwarded.clear_debug_on_next_call();
    let token = e.vm.backtrace.frames[mark].token;
    e.vm.backtrace.set_debug_on_exit(token, true);
    e.vm.debug_hooks_changed();
    e.vm.profiler.debug_call = true;
    e.budget = 0;
}
/// GNU call_debugger's form: `debugger' on ARGS under its bindings. Run as
/// the engine's own code, it adds no frame under the debugger's.
fn debugger_form(vm: &Vm, args: &[Value]) -> Form {
    let sym = |name: &str| Form::Sym(name.into());
    let bind = |name: &str, value: &str| Form::List(vec![sym(name), sym(value)]);
    let mut call = vec![sym("funcall"), sym("debugger")];
    call.extend(args.iter().map(|&arg| quoted(vm, arg)));
    let bindings = Form::List(vec![bind("debugger-may-continue", "t"), bind("inhibit-redisplay", "nil"),
                                   bind("inhibit-debugger", "t"), bind("inhibit-changing-match-data", "nil")]);
    let note = Form::List(vec![sym("setq"), sym("debug-on-next-call"), sym("nil"),
                               sym("internal-when-entered-debugger"), sym("num-nonmacro-input-events")]);
    Form::List(vec![sym("progn"), note, Form::List(vec![sym("let"), bindings, Form::List(call)])])
}
/// `nested_then', with two hundred levels borrowed. The reserve is taken at
/// the engine's depth, which `eval_depth' learns only at a nested boundary.
fn nested_with_room<'a, 'b, B: Plain>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, call: Nested<'_>, b: B,
    then: impl FnOnce(&mut Engine<'a>, B, Value) + Copy + 'static) -> bool {
    let token = m.leaf((), |e, ()| {
        let saved_depth = std::mem::replace(&mut e.vm.eval_depth, e.depth_base + e.frames.len());
        let token = e.vm.max_ensure_room(200);
        e.vm.eval_depth = saved_depth;
        token
    });
    let out = nested_then(m, gc, call, b, then);
    m.leaf(token, |e, token| if let Some(old) = token { e.vm.restore_stack_limits(old) });
    out
}
/// Run the debugger's FORM (`debugger_form`) with this activation
/// suspended, borrowing two hundred levels; its value goes to the
/// destination register of the instruction at DEST, when given. False with
/// its escape pending.
fn debugger_lent<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, form: &Form, dest: Option<Pc<'_>>) -> bool {
    nested_with_room(m, gc, Nested::Form(form), dest.map(|pc| pc.a()), |e, dest, value| {
        if let Some(dest) = dest { e.set_reg(dest, value); }
    })
}
/// `debug-on-next-call''s stop, at the first poll of the call it armed.
#[cold]
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn debug_on_call(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    debug_assert_eq!(r, e.regs, "a poll runs in the current frame");
    e.vm.profiler.debug_call = false;
    let code = e.vm.intern("lambda");
    let form = Box::new(debugger_form(e.vm, &[code]));
    // SAFETY: PC is the poll's instruction, in this engine's code.
    orchestrate(e, (unsafe { Pc::new(pc) }, form), debugged_call, continue_at)
}
#[inline(never)]
fn debugged_call<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, (pc, form): (Pc<'x>, Box<Form>)) -> Continue<'x> {
    if debugger_lent(&mut m, gc.by_ref(), &form, None) { Continue::Poll(pc) } else { Continue::Stop }
}
/// GNU funcall's debug-on-exit: the debugger gets `(exit VALUE)' as a
/// flagged activation returns normally, and its value is returned instead.
#[cold]
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn debug_on_exit(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst, mark: usize) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        debug_assert_eq!(r, e.regs, "a return runs in the current frame");
        e.vm.backtrace.drop_exit_flags_from(mark);
        e.vm.debug_hooks_changed();
        let exit = e.vm.intern("exit");
        let value = *r.add((*pc).a);
        let form = Box::new(debugger_form(e.vm, &[exit, value]));
        orchestrate(e, (Pc::new(pc), form), debugged_exit, continue_at)
    }
}
#[inline(never)]
fn debugged_exit<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, (pc, form): (Pc<'x>, Box<Form>)) -> Continue<'x> {
    if debugger_lent(&mut m, gc.by_ref(), &form, Some(pc)) { Continue::Return(pc) } else { Continue::Stop }
}
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn enter_native(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst,
    target: Value, start: usize, argc: usize) {
    let mark = if let Some(mark) = e.native_mark.take() {
        e.vm.backtrace.resolve_pending(target);
        mark
    } else {
        // SAFETY: R and PC as the function's contract (`# Safety`).
        unsafe { trace_call::<false>(e, r, pc, target, start, argc) }
    };
    e.native_mark = Some(e.apply_mark.take().unwrap_or(mark));
}
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn enter_apply(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst, start: usize, argc: usize) {
    let target = Value::reference(Primitive::Apply as usize, 6);
    let mark = if let Some(mark) = e.native_mark.take() {
        e.vm.backtrace.resolve_pending(target);
        mark
    } else {
        // SAFETY: R and PC as the function's contract (`# Safety`).
        unsafe { trace_call::<false>(e, r, pc, target, start, argc) }
    };
    // A callee's physical activation owns its whole native apply chain. This
    // keeps genuine apply operations visible until the final callee leaves.
    e.apply_mark.get_or_insert(mark);
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn call(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe { invoke(e, r, pc, *r.add((*pc).b), (*pc).c, (*pc).d) }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn direct(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
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
            if !depth_room(&mut e) { return Value::UNBOUND; }
            let Some(r) = frame_room(&mut e, r) else { return fail(&mut e, "register storage overflow"); };
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
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn direct_generic(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let en: &mut Engine<'_> = &mut e;
        let i = &*pc;
        // Function-cell equality is an authoritative guard, including redefinition
        // back to the original primitive; never assume a sealed symbol table.
        let slot = en.vm.slots.get_unchecked(i.b);
        let cache = en.vm.caches.get_unchecked_mut(i.c);
        if cache.version != slot.version {
            if en.vm.macros.contains(&i.b) && !en.vm.expanding.contains(&i.b) {
                return fail(
                    en,
                    "invalid function: macro dependency changed; resubmit source",
                );
            }
            cache.version = slot.version;
            cache.target = slot.function;
        }
        let target = cache.target;
        if target == Value::UNBOUND || target == Value::NIL {
            return raise(en, "void-function", &[Value::reference(i.b + 1, 7)]);
        }
        // An autoload cell is resolved from its symbol, which it must name.
        if target.tag() == 7 || target.tag() == 3 && (en.vm.autoload_object(target) || lambda_list(en, target)) {
            return invoke_full(e, r, pc, Value::reference(i.b + 1, 7), i.d, i.e);
        }
        // GNU eval.c:3158-3160 names the called SYMBOL, not its cell's value.
        if !matches!(target.tag(), 4 | 6) {
            let condition = uncallable_condition(en, target);
            return raise(en, condition, &[Value::reference(i.b + 1, 7)]);
        }
        // Refresh an invalidated speculation lazily; ineligible closures retain
        // the existing function-id cache instead of resolving on every call.
        if target.tag() == 4 && en.entries[i.site].world_version != 0 {
            return resolve_entry(e, r, pc, target, i.d, i.e);
        }
        invoke(e, r, pc, target, i.d, i.e)
    }
}
#[inline(always)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn invoke(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let en: &mut Engine<'_> = &mut e;
        if target.tag() != 4 {
            let entry = *en.entries.get_unchecked((*pc).site);
            if entry.target == target {
                if entry.primitive.is_some() {
                    return cached_primitive(e, r, pc, target, start, argc);
                }
                if entry.simple_arity == LEAF_ENTRY + argc {
                    return leaf_call(e, r, pc, target, start, argc);
                }
            }
            return resolve_entry(e, r, pc, target, start, argc);
        }
        let Some(closure) = en.vm.closures.get(target.payload() as usize) else {
            return fail(en, "invalid closure reference");
        };
        let function = closure.function;
        let entry = *en.entries.get_unchecked((*pc).site);
        if entry.function != function || entry.entry.is_null() {
            return resolve_entry(e, r, pc, target, start, argc);
        }
        if !depth_room(en) { return Value::UNBOUND; }
        let Some(r) = frame_room(en, r) else { return fail(en, "register storage overflow"); };
        if entry.simple_arity == argc {
            let new = r.add(en.vm.functions.get_unchecked(en.function).regs.max(1));
            let mark = call_mark(en, r, pc, target, start, argc);
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
        let f = en.vm.functions.get_unchecked(function);
        if argc < f.required || (!f.rest && argc > f.arity) {
            enter_native(en, r, pc, target, start, argc);
            return closure_arity_error(en, function, argc);
        }
        let new = r.add(en.vm.functions.get_unchecked(en.function).regs.max(1));
        let dst = new.add(f.captures);
        let positional = argc.min(f.arity);
        // The arguments may lie where the new frame goes (`apply`'s spread
        // window once storage has widened): read them all before writing
        // the frame (the backtrace entry, then rest values, then the
        // positional ones by memmove) and only then write the captures.
        let mark = call_mark(en, r, pc, target, start, argc);
        let f = en.vm.functions.get_unchecked(function);
        let closure = &en.vm.closures[target.payload() as usize];
        let rest = if f.rest {
            let begin = en.rest.values.len();
            for i in positional..argc {
                en.rest.values.push(*r.add(start + i));
            }
            Some(OpaqueRest {
                reg: f.captures + f.arity,
                start: begin,
                end: en.rest.values.len(),
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
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn resolve_entry(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let en: &mut Engine<'_> = &mut e;
        let mut r = r;
        if target.tag() == 4 {
            let Some(closure) = en.vm.closures.get(target.payload() as usize) else {
                return fail(en, "invalid closure reference");
            };
            let function = closure.function;
            if en.vm.functions[function].lazy.is_some() {
                // No collection runs in it (`force_lazy`), so `target` and the
                // argument registers keep their indices; only `r` is rebased.
                match force_lazy(en, r, function) {
                    Some(rebased) => r = rebased,
                    None => return Value::UNBOUND,
                }
            }
            let closure = &en.vm.closures[target.payload() as usize];
            let world_version = if matches!((*pc).op, Op::Direct(..))
                && closure.captures.is_empty()
                && en.vm.functions[function].required == en.vm.functions[function].arity
                && !en.vm.functions[function].rest
                && en.vm.slots[(*pc).b].function == target
                && !en.vm.macros.contains(&(*pc).b)
                && !en.vm.profiler.next_call
            {
                en.vm.world_version
            } else {
                0
            };
            if let Err(error) = lower_function(
                en.vm,
                &mut en.code,
                &mut en.entries,
                &mut en.inline_bodies,
                &mut en.free_entries,
                function,
            ) {
                en.fatal_image_failure = true;
                en.error = Some(error);
                return Value::UNBOUND;
            }
            en.inline_bodies[(*pc).site] = if world_version != 0 {
                #[cfg(test)]
                if en.vm.disable_inline {
                    None
                } else {
                    inline_body(en.vm, function)
                }
                #[cfg(not(test))]
                inline_body(en.vm, function)
            } else {
                None
            };
            *en.entries.get_unchecked_mut((*pc).site) = EntryCache {
                world_version,
                // A function id is stable through moving GC. Never retain one
                // particular environment as the cache key or copy its captures.
                target: Value::UNBOUND,
                function,
                entry: en.code.get_unchecked(function).as_ptr(),
                primitive: None,
                simple_arity: {
                    let f = &en.vm.functions[function];
                    if f.captures == 0 && !f.rest && f.required == f.arity {
                        f.arity
                    } else {
                        usize::MAX
                    }
                },
            };
            return invoke(e, r, pc, target, start, argc);
        }
        if let Some(d) = prim::generated(target) {
            let Some(leaf) = prim::site_leaf(d, argc) else {
                return enter_generated(e, r, pc, target, start, argc);
            };
            en.inline_bodies[(*pc).site] = None;
            *en.entries.get_unchecked_mut((*pc).site) = EntryCache {
                world_version: 0,
                target,
                function: leaf as usize,
                entry: std::ptr::null(),
                simple_arity: LEAF_ENTRY + argc,
                primitive: None,
            };
            return leaf_call(e, r, pc, target, start, argc);
        }
        if target.tag() == 6 {
            let Some(p) = prim::legacy(target) else {
                return fail(en, "invalid primitive reference");
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
                en.inline_bodies[(*pc).site] = None;
                *en.entries.get_unchecked_mut((*pc).site) = EntryCache {
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
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn invoke_full(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let function = target;
        // Out of line: the alias walk's address-taken scratch would stop
        // every tail dispatch below, so a funcall or apply in a loop grew
        // the native stack.
        let Some(target) = t2_resolve(&mut e, target) else {
            return Value::UNBOUND;
        };
        if target == Value::reference(Primitive::Apply as usize, 6) {
            return apply(e, r, pc, start, argc);
        }
        if target == Value::reference(Primitive::Throw as usize, 6)
            || target == Value::reference(Primitive::Signal as usize, 6)
        {
            enter_native(&mut e, r, pc, target, start, argc);
            let signal = target == Value::reference(Primitive::Signal as usize, 6);
            if !(argc == 2 || (signal && argc == 1)) {
                return fail(&mut e, "wrong primitive arity");
            }
            if signal {
                // eval.c Fsignal/signal_or_quit: an error object with DATA
                // omitted re-signals itself; nil with non-list DATA is `error'.
                let data = if argc == 2 { *r.add(start + 1) } else { Value::NIL };
                let mut condition = *r.add(start);
                if condition.tag() == 3 && data == Value::NIL {
                    let Ok((condition, data)) = e.vm.cons_pair(condition) else { return fail(&mut e, "invalid error object") };
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
                    return wrong_type(&mut e, "symbolp", condition);
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
            enter_native(&mut e, r, pc, target, start, argc);
            debug_assert_eq!(r, e.regs, "a call runs in the current frame");
            // The function first: the call is retried from it (`retry_call`).
            let retry = Box::new((Pc::new(pc), e.vm.root_frame(&[function, target]), (start, argc), false));
            return orchestrate(e, retry, autoload_call, continue_at);
        }
        if lambda_list(&mut e, target) {
            enter_native(&mut e, r, pc, target, start, argc);
            if let Some(failure) = lambda_list_failure(e.vm, target, std::slice::from_raw_parts(r.add(start), argc).to_vec()) {
                let raised = lambda_list_frame(e.vm, failure);
                return orchestrate(e, raised, lambda_list_raise, continue_at);
            }
            debug_assert_eq!(r, e.regs, "a call runs in the current frame");
            let retry = Box::new((Pc::new(pc), e.vm.root_frame(&[target]), (start, argc), false));
            return orchestrate(e, retry, lambda_call, continue_at);
        }
        if target.tag() != 4 {
            return {
                let condition = uncallable_condition(&mut e, target);
                raise(&mut e, condition, &[function])
            };
        }
        invoke(e, r, pc, target, start, argc)
    }
}
/// Cold apply adapter. Its bounded scratch window is disjoint from all frames.
/// Arguments are copied into the callee before the next handler executes; nested
/// calls therefore never retain a pointer into the shared scratch window.
#[cold]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn apply(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        enter_apply(&mut e, r, pc, start, argc);
        let base = r.offset_from(e.vm.stack.as_ptr()) as usize;
        // Out of line: the Result's error drop would stop the tail dispatch.
        let Some((r, target, offset, count)) = t2_prepare_apply(&mut e, r, base + start, argc) else {
            return Value::UNBOUND;
        };
        e.apply_function = Some(target);
        invoke_full(e, r, pc, target, offset - base, count)
    }
}
#[inline(always)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
#[expect(clippy::too_many_arguments, reason = "the handler ABI: every operand stays in a register")]
unsafe fn enter_frame(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    new: *mut Value,
    function: usize,
    rest: Option<OpaqueRest>,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let mark = call_mark(&mut e, r, pc, target, start, argc);
        enter_frame_marked(e, r, pc, new, function, rest, mark)
    }
}
/// The backtrace entry of the call about to enter: its arguments are read
/// here, so a caller whose arguments may lie under the new frame (`apply`'s
/// spread window) takes it before writing the frame.
#[inline(always)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn call_mark(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst, target: Value, start: usize, argc: usize) -> usize {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        if let Some(mark) = e.native_mark.take() {
            // The activation a primitive (`funcall') entered becomes this
            // Lisp call's: it is the call `debug-on-next-call' stops.
            e.vm.backtrace.resolve_pending(target);
            if e.vm.profiler.active { arm_debug_on_call(e, mark); }
            mark
        } else {
            trace_call::<true>(e, r, pc, target, start, argc)
        }
    }
}
#[inline(always)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn enter_frame_marked(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    new: *mut Value,
    function: usize,
    rest: Option<OpaqueRest>,
    mark: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let en: &mut Engine<'_> = &mut e;
        // Captures/arguments have already been copied, including on lower failure.
        let end = new.offset_from(en.vm.stack.as_ptr()) as usize + en.vm.functions[function].regs;
        en.stack_used = en.stack_used.max(end);
        // The function-id entry cache guarantees lowering before frame entry.
        en.frames.push(Frame {
            pc: pc.add(1),
            regs: r,
            dest: (*pc).a as u32,
            function: en.function as u32,
            dynamic: en.dynamic,
            controls_base: en.controls_base,
            mark: en.apply_mark.take().unwrap_or(mark),
        });
        // An empty window needs no record: the rest register already holds nil.
        if let Some(rest) = rest
            && rest.start != rest.end
        {
            en.rest.enter(en.frames.len(), rest.reg, rest.start, rest.end);
        }
        en.controls_base = en.controls.len();
        en.regs = new;
        en.function = function;
        en.dynamic = en.vm.dynamic.len();
        let pc = en.code.get_unchecked(function).as_ptr();
        en.budget -= 1;
        if en.budget < 0 {
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
        kernel: direct,
    }
}
static K_CODE: [Inst; 3] = [k_inst(t2_call), k_inst(t2_resume), k_inst(ret)];
fn k_count(v: Value) -> usize {
    v.integer_value().expect("continuation count register") as usize
}
/// Registers used by the continuation activation at `k`.
/// # Safety
/// K is a continuation activation's registers (`take_step`), its counts in
/// place.
unsafe fn k_width(k: *mut Value) -> usize {
    // SAFETY: K as the function's contract.
    unsafe { K_SAVED + k_count(*k.add(K_SAVED_N)) + k_count(*k.add(K_ARGS_N)) }
}
/// Where a handler continues after a Step, as plain data in two words, the
/// registers the current frame's: a step's argument vectors are owned and
/// dropped inside the non-inlined helpers below, never live across `next`
/// (which would retain a native frame per callback), and an orchestration
/// returns one (`orchestrate`, `continue_at` its continuation).
enum Continue<'x> {
    At(Pc<'x>),
    Poll(Pc<'x>),
    /// The instruction after this one.
    After(Pc<'x>),
    /// A normal return at the instruction (`ret`).
    Return(Pc<'x>),
    Stop,
    /// `garbage-collect' at the instruction (`collect_explicit`).
    Collect(Pc<'x>),
    /// `romacs--collect-idle' at the instruction (`collect_idle`).
    CollectIdle(Pc<'x>),
    /// A reading generated primitive (`read_body`): at the instruction, the
    /// primitive on the ARGC arguments at register offset START.
    Read(Box<(Pc<'x>, Imm, usize, usize)>),
    /// A call retried at the instruction (`retry`): the function rooted at
    /// the frame's first index, the START and ARGC arguments, and whether
    /// the call came from a continuation's activation.
    Retry(Box<(Pc<'x>, RootFrame, (usize, usize), bool)>),
}
// SAFETY: instruction addresses (`Pc`), immediates, a root frame and indices.
unsafe impl Plain for Continue<'_> {}
/// Dispatch TO in the current frame.
#[inline(always)]
fn continue_at(e: Eng<'_, '_>, to: Continue<'_>) -> Value {
    // The plain dispatches first, as a copy: `to` (whose boxed variants have
    // drop glue) is gone before the call, which then can be a sibling call.
    let hot = match to.hot() {
        Ok(hot) => hot,
        Err(to) => return continue_lent(e, to),
    };
    let r = e.regs;
    // SAFETY: a `Pc` names an instruction of this engine's code (`Pc::new`),
    // and `e.regs` is the frame it runs in.
    unsafe {
        match hot {
            Hot::At(pc) => next(e, r, pc.get()),
            Hot::Poll(pc) => poll_resume(e, r, pc.get()),
            Hot::After(pc) => advance(e, r, pc.get()),
            Hot::Return(pc) => ret(e, r, pc.get()),
            Hot::Stop => Value::UNBOUND,
        }
    }
}
/// The continuations `continue_at` dispatches in place.
#[derive(Clone, Copy)]
enum Hot<'x> {
    At(Pc<'x>),
    Poll(Pc<'x>),
    After(Pc<'x>),
    Return(Pc<'x>),
    Stop,
}
impl<'x> Continue<'x> {
    /// The dispatch made in place, or the lend this is.
    #[inline(always)]
    fn hot(self) -> std::result::Result<Hot<'x>, Self> {
        Ok(match self {
            Continue::At(pc) => Hot::At(pc),
            Continue::Poll(pc) => Hot::Poll(pc),
            Continue::After(pc) => Hot::After(pc),
            Continue::Return(pc) => Hot::Return(pc),
            Continue::Stop => Hot::Stop,
            to => return Err(to),
        })
    }
}
/// The continuations that are lends: collections, a reader body, and a call
/// retried after a load or a lambda list's compile.
#[cold]
#[inline(never)]
fn continue_lent(e: Eng<'_, '_>, to: Continue<'_>) -> Value {
    match to {
        Continue::Collect(pc) => orchestrate(e, pc, collect_explicit, continue_at),
        Continue::CollectIdle(pc) => orchestrate(e, pc, collect_idle, continue_at),
        Continue::Read(read) => orchestrate(e, read, read_body, continue_at),
        Continue::Retry(retry) => retry_call(e, retry),
        to => continue_at(e, to),
    }
}
/// Call FUNCTION (the frame's first value) again from the instruction, as the
/// call site or the continuation would have.
#[inline(never)]
fn retry_call(mut e: Eng<'_, '_>, retry: Box<(Pc<'_>, RootFrame, (usize, usize), bool)>) -> Value {
    let (pc, function, (start, argc), t2) = retry_target(&mut e, retry);
    let r = e.regs;
    // SAFETY: as `continue_at`.
    unsafe { if t2 { t2_dispatch(e, r, pc.get(), function, start, argc) } else { t2_invoke_full(e, r, pc.get(), function, start, argc) } }
}
/// RETRY's call, its frame released: out of line, so the box and the frame
/// are gone before `retry_call`'s dispatch.
#[inline(never)]
#[expect(clippy::boxed_local, reason = "a one-word handoff keeps `retry_call`'s frame free for its sibling call")]
fn retry_target<'x>(e: &mut Engine<'_>, retry: Box<(Pc<'x>, RootFrame, (usize, usize), bool)>) -> (Pc<'x>, Value, (usize, usize), bool) {
    let (pc, frame, call, t2) = *retry;
    let function = e.vm.rooted(&frame, 0);
    e.vm.drop_frame(frame);
    (pc, function, call, t2)
}
/// Enter a generated primitive from a call site (a caller's instruction or
/// `K_CODE[0]`, when the callback is itself a generated primitive).
#[cold]
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn enter_generated(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let to = generated_step(&mut e, r, pc, target, start, argc);
        continue_at(e, to)
    }
}
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn generated_step(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Continue<'static> {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        enter_native(e, r, pc, target, start, argc);
        let base = r.offset_from(e.vm.stack.as_ptr()) as usize;
        // The reader runs Lisp: a lend, which the dispatch makes.
        if matches!(prim::name(target), Some("read" | "read-from-string")) {
            debug_assert_eq!(r, e.regs, "a call runs in the current frame");
            return Continue::Read(Box::new((Pc::new(pc), Imm::new(e.vm, target).expect("a primitive"), base + start, argc)));
        }
        let step = e.vm.call_generated(target, base + start, argc);
        // A region attached under id pressure (`frozen.rs`), or big payloads
        // made (`Vm::collect_poll'): poll, and collect if due, soon.
        if e.vm.region_collect || (e.vm.collect_poll && e.vm.may_collect(1)) { e.budget = 0; }
        take_step(e, r, pc, step)
    }
}
/// A reading generated primitive (`read', `read-from-string'): its body
/// runs with this engine's code suspended and its registers in place, as
/// the reader may call Lisp; its step then continues the call.
#[inline(never)]
#[expect(clippy::boxed_local, reason = "an orchestration's input fits two words (`orchestrate`), so its caller keeps its sibling calls")]
fn read_body<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, read: Box<(Pc<'x>, Imm, usize, usize)>) -> Continue<'x> {
    let (pc, target, start, argc) = *read;
    let saved_depth = m.leaf((), |e, ()| {
        e.vm.propagated_escape = None;
        e.vm.propagated_escape_hooked = false;
        e.vm.propagated_eval_depth = None;
        std::mem::replace(&mut e.vm.eval_depth, e.depth_base + e.frames.len())
    });
    lend::read(&mut m, gc.by_ref(), (target, start, argc), (pc, saved_depth), |e, (pc, saved_depth), step| {
        if e.vm.region_collect || (e.vm.collect_poll && e.vm.may_collect(1)) { e.budget = 0; }
        e.vm.eval_depth = saved_depth;
        e.fatal_image_failure |= e.vm.execution_fatal;
        e.code.resize_with(e.vm.functions.len(), Vec::new);
        let (r, width) = (e.regs, e.vm.register_width);
        // SAFETY: R is this engine's current frame, in the register stack, and
        // PC the reading call's instruction, in this engine's code.
        unsafe {
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
            take_step(e, r, pc.get(), step)
        }
    })
}
/// Act on a Step produced at `pc`: a call site, or `K_CODE[1]` when a
/// continuation ran in its own activation (which a further Call then reuses).
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn take_step(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    step: std::result::Result<prim::Step, Option<(Value, Value)>>,
) -> Continue<'static> {
    // SAFETY: R and PC as the function's contract (`# Safety`).
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
            // Collections are lends, which the dispatch makes (`continue_at`).
            Ok(prim::Step::Collect) => {
                leave_native(e);
                debug_assert_eq!(r, e.regs, "a step runs in the current frame");
                return Continue::Collect(Pc::new(pc));
            }
            Ok(prim::Step::CollectIdle) => {
                leave_native(e);
                debug_assert_eq!(r, e.regs, "a step runs in the current frame");
                return Continue::CollectIdle(Pc::new(pc));
            }
            Ok(prim::Step::Safepoint) => {
                leave_native(e);
                *r.add((*pc).a) = Value::NIL;
                debug_assert_eq!(r, e.regs, "a step runs in the current frame");
                return Continue::Poll(Pc::new(pc.add(1)));
            }
            Ok(prim::Step::Done(value)) => {
                leave_native(e);
                if std::ptr::eq(pc, &K_CODE[1]) && e.controls.len() > e.controls_base
                    && e.controls.last().is_some_and(|c| matches!(c.kind, ControlKind::HandlerBind { .. })) {
                    let control = e.controls.pop().unwrap();
                    release_control(e, &control);
                }
                *r.add((*pc).a) = value;
                debug_assert_eq!(r, e.regs, "a step continues in the current frame");
                return Continue::At(Pc::new(pc.add(1)));
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
                .is_some_and(|(limit, end)| end <= limit && end <= e.vm.stack.len() - e.vm.spread_reserve)
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
            e.frames.push(Frame {
                pc: pc.add(1),
                regs: r,
                dest: (*pc).a as u32,
                function: e.function as u32,
                dynamic: e.dynamic,
                controls_base: e.controls_base,
                mark: e.native_mark.take().unwrap_or(NO_MARK),
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
            Continue::Poll(Pc::new(K_CODE.as_ptr()))
        } else {
            debug_assert_eq!(k, e.regs, "a step continues in the current frame");
            Continue::At(Pc::new(K_CODE.as_ptr()))
        }
    }
}
fn handler_bind_return(_: &mut prim::Leaf<'_>, value: Value, _: &[Value]) -> prim::Next {
    Ok(prim::Step::Done(value))
}

/// Run the continuation of the activation at `k` on K_RES and its saved
/// registers, copied for the duration of a leaf call that cannot collect.
#[inline(never)]
/// # Safety
/// K is the continuation activation that is the current frame, and PC its
/// instruction (`K_CODE`).
unsafe fn resume_step(e: &mut Engine<'_>, k: *mut Value, pc: *const Inst) -> Continue<'static> {
    // SAFETY: K and PC as the function's contract.
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
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn t2_call(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let saved = k_count(*r.add(K_SAVED_N));
        let argc = k_count(*r.add(K_ARGS_N));
        t2_dispatch(e, r, pc, *r.add(K_F), K_SAVED + saved, argc)
    }
}
/// `K_CODE[1]`: run the continuation on K_RES and the saved registers.
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn t2_resume(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let to = resume_step(&mut e, r, pc);
        continue_at(e, to)
    }
}
/// Ordinary call semantics from the continuation activation. It mirrors
/// `invoke_full` but never touches the site caches, and places a callee frame
/// after the activation's own registers rather than the caller function's.
#[cold]
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn t2_dispatch(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let function = target;
        let Some(target) = t2_resolve(&mut e, target) else {
            return Value::UNBOUND;
        };
        if target.tag() == 3 && e.vm.autoload_object(target) {
            enter_native(&mut e, r, pc, target, start, argc);
            debug_assert_eq!(r, e.regs, "a call runs in the current frame");
            // The function first: the call is retried from it (`retry_call`).
            let retry = Box::new((Pc::new(pc), e.vm.root_frame(&[function, target]), (start, argc), true));
            return orchestrate(e, retry, autoload_call, continue_at);
        }
        if lambda_list(&mut e, target) {
            enter_native(&mut e, r, pc, target, start, argc);
            if let Some(failure) = lambda_list_failure(e.vm, target, std::slice::from_raw_parts(r.add(start), argc).to_vec()) {
                let raised = lambda_list_frame(e.vm, failure);
                return orchestrate(e, raised, lambda_list_raise, continue_at);
            }
            debug_assert_eq!(r, e.regs, "a call runs in the current frame");
            let retry = Box::new((Pc::new(pc), e.vm.root_frame(&[target]), (start, argc), true));
            return orchestrate(e, retry, lambda_call, continue_at);
        }
        if target.tag() == 4 {
            return t2_enter_closure(e, r, pc, target, start, argc);
        }
        if prim::generated(target).is_some() {
            return enter_generated(e, r, pc, target, start, argc);
        }
        if target.tag() != 6 {
            return {
                let condition = uncallable_condition(&mut e, target);
                raise(&mut e, condition, &[function])
            };
        }
        let Some(p) = prim::legacy(target) else {
            return fail(&mut e, "invalid primitive reference");
        };
        let base = r.offset_from(e.vm.stack.as_ptr()) as usize;
        match p {
            Primitive::Apply => {
                enter_apply(&mut e, r, pc, start, argc);
                let Some((r, target, offset, count)) = t2_prepare_apply(&mut e, r, base + start, argc) else {
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
                enter_native(&mut e, r, pc, target, start, argc);
                let Some(value) = primitive_value(&mut e, p, base + start, argc, primitive_target(p))
                else {
                    return Value::UNBOUND;
                };
                leave_native(&mut e);
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
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn t2_invoke_full(
    e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe { invoke_full(e, r, pc, target, start, argc) }
}
/// Spread `apply`'s arguments; `None` after recording the error.  A spread
/// wider than the stack's reserved tail widens the reserve first, rebasing
/// every activation (offsets are unchanged), so the caller's registers move.
#[cold]
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`), holding the
/// registers the caller names.
unsafe fn t2_prepare_apply(
    e: &mut Engine<'_>,
    r: *mut Value,
    start: usize,
    argc: usize,
) -> Option<(*mut Value, Value, usize, usize)> {
    let mut r = r;
    loop {
        // prepare_apply writes a disjoint scratch window at the end.
        match e.vm.prepare_apply(start, argc) {
            Ok(Ok(window)) => {
                match e.apply_scratch.iter_mut().find(|(offset, _)| *offset == window.1) {
                    Some((_, count)) => *count = (*count).max(window.2),
                    None => e.apply_scratch.push((window.1, window.2)),
                }
                return Some((r, window.0, window.1, window.2));
            }
            Ok(Err(count)) => {
                let Some(reserve) = count.checked_next_power_of_two() else {
                    fail(e, "register storage overflow");
                    return None;
                };
                e.vm.spread_reserve = reserve;
                let width = e.vm.register_width;
                // SAFETY: R as the function's contract (`# Safety`).
                r = unsafe { widen_registers(e, r, width) }?;
            }
            Err(error) => {
                e.error = Some(error);
                return None;
            }
        }
    }
}
/// `invoke`'s general closure entry, with the callee after the activation.
#[cold]
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn t2_enter_closure(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let en: &mut Engine<'_> = &mut e;
        let Some(closure) = en.vm.closures.get(target.payload() as usize) else {
            return fail(en, "invalid closure reference");
        };
        let function = closure.function;
        if !depth_room(en) { return Value::UNBOUND; }
        let Some(r) = frame_room(en, r) else { return fail(en, "register storage overflow"); };
        let mut r = r;
        if en.vm.functions[function].lazy.is_some() {
            match force_lazy(en, r, function) {
                Some(rebased) => r = rebased,
                None => return Value::UNBOUND,
            }
        }
        if let Err(error) = lower_function(
            en.vm,
            &mut en.code,
            &mut en.entries,
            &mut en.inline_bodies,
            &mut en.free_entries,
            function,
        ) {
            en.fatal_image_failure = true;
            en.error = Some(error);
            return Value::UNBOUND;
        }
        let f = &en.vm.functions[function];
        if argc < f.required || (!f.rest && argc > f.arity) {
            enter_native(en, r, pc, target, start, argc);
            return closure_arity_error(en, function, argc);
        }
        let new = r.add(k_width(r));
        let dst = new.add(f.captures);
        let positional = argc.min(f.arity);
        // As in the cached entry: read every argument, the backtrace's
        // included, before writing the frame, which may overlap `apply`'s
        // spread window after widening.
        let mark = call_mark(en, r, pc, target, start, argc);
        let f = &en.vm.functions[function];
        let closure = &en.vm.closures[target.payload() as usize];
        let rest = if f.rest {
            let begin = en.rest.values.len();
            for i in positional..argc {
                en.rest.values.push(*r.add(start + i));
            }
            Some(OpaqueRest {
                reg: f.captures + f.arity,
                start: begin,
                end: en.rest.values.len(),
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
        DeepEqual | Min | Append => Vm::eval_data_primitive,
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
        Car | Cdr | Null | Eq | Memq | Assq | Nth | Setcar | Setcdr => {
            Vm::traversal_primitive
        }
        Message => Vm::primitive,
        // Only `provide' and `featurep' come here (the rest route through
        // `primitive', a lend); `provide''s after-load hooks run deferred.
        Load | Require | Provide | Featurep | Eval | Macroexpand | AutoloadDoLoad | ReadHostEvent | FormatModeLine => |vm, p, start, n| {
            let args = vm.stack[start..start + n].to_vec();
            vm.defer(|vm| vm.loader_primitive(p, &args))
        },
        Get | Put | RawPropertiesAt | GetCharProperty | BufferSubstring
        | InsertInherit | KillNew | CurrentKill | YankPart | YankFinalize | YankPropertyRun
        | YankFontLockFace | YankCategory | GetProperty => Vm::annotation_primitive,
        p if (List as usize..=Memq as usize).contains(&(p as usize)) => Vm::bootstrap_primitive,
        p if (p as usize) >= CurrentBuffer as usize => Vm::buffer_primitive,
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
            // travels behind the write's text, as does GNU's read-only
            // error with its object behind a frozen write's (`frozen.rs`).
            if (matches!(p, Primitive::Set | Primitive::Makunbound | Primitive::DefvarSetDefault | Primitive::Defalias) || error == crate::frozen::READ_ONLY)
                && let Some(escape) = e.vm.propagated_escape.take() {
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
/// `EntryCache::simple_arity' of a leaf site, plus its argument count; the
/// leaf's body is in `function' (`prim::site_leaf').
const LEAF_ENTRY: usize = usize::MAX / 2;
/// A leaf through its site: no activation unless it fails, as nothing can see
/// one while it runs. Whatever watches calls keeps the full path. Six
/// register arguments, for the reason `cached_primitive' gives.
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn leaf_call(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let en: &mut Engine<'_> = &mut e;
        if en.native_mark.is_some() || en.apply_mark.is_some() || en.vm.profiler.active
            || en.vm.backtrace.observing() || en.vm.fence.is_some()
        {
            return enter_generated(e, r, pc, target, start, argc);
        }
        let leaf: prim::LeafBody = std::mem::transmute(en.entries.get_unchecked((*pc).site).function);
        en.vm.site_leaf = (pc as usize, r as usize);
        let value = en.vm.leaf_value(leaf, r.add(start), argc, &mut en.leaf_fault);
        en.vm.site_leaf = (0, 0);
        if value == Value::UNBOUND && en.leaf_fault.is_some() {
            return leaf_failed(e, r, pc, target, start, argc);
        }
        if en.vm.region_collect || (en.vm.collect_poll && en.vm.may_collect(1)) {
            en.budget = 0;
        }
        *r.add((*pc).a) = value;
        advance(e, r, pc)
    }
}
/// The call a failed leaf made, recorded now: its arguments are where they
/// were, and the fault is reported once, as if the activation had been there.
#[cold]
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn leaf_failed_step(
    e: &mut Engine<'_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Continue<'static> {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        enter_native(e, r, pc, target, start, argc);
        let fault = e.leaf_fault.take().expect("a failed leaf's fault");
        let step = e.vm.leaf_step(fault);
        take_step(e, r, pc, step)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn leaf_failed(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let to = leaf_failed_step(&mut e, r, pc, target, start, argc);
        continue_at(e, to)
    }
}
/// A primitive through the site's entry cache, which holds its call. Six
/// register arguments: a seventh on the stack would stop every caller from
/// tail-calling it, so each funcall or apply of a primitive kept a frame.
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn cached_primitive(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let call = e.entries.get_unchecked((*pc).site).primitive.expect("cached primitive call");
        enter_native(&mut e, r, pc, target, start, argc);
        let base = r.offset_from(e.vm.stack.as_ptr()) as usize;
        let p = prim::legacy(target).expect("legacy primitive");
        let Some(value) = primitive_value(&mut e, p, base + start, argc, call) else {
            return Value::UNBOUND;
        };
        leave_native(&mut e);
        *r.add((*pc).a) = value;
        advance(e, r, pc)
    }
}
#[cold]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn primitive(
    mut e: Eng<'_, '_>,
    r: *mut Value,
    pc: *const Inst,
    target: Value,
    start: usize,
    argc: usize,
) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let en: &mut Engine<'_> = &mut e;
        enter_native(en, r, pc, target, start, argc);
        let base = r.offset_from(en.vm.stack.as_ptr()) as usize;
        let p = prim::legacy(target).expect("legacy primitive");
        if matches!(p, Primitive::Load | Primitive::Require | Primitive::Eval | Primitive::Macroexpand | Primitive::AutoloadDoLoad | Primitive::ReadHostEvent | Primitive::FormatModeLine | Primitive::MakeInterpretedClosure) {
            debug_assert_eq!(r, en.regs, "a call runs in the current frame");
            return orchestrate(e, (Pc::new(pc), Box::new((p, base + start, argc))), loader_body, continue_at);
        }
        let mut signal = None;
        match en
            .vm
            .primitive_call(p, base + start, argc, &mut en.error, &mut signal)
        {
            Some(value) => {
                leave_native(en);
                *r.add((*pc).a) = value;
                advance(e, r, pc)
            }
            None => {
                if let Some((condition, data)) = signal {
                    en.escape = Some(Escape::Signal(condition, data));
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
/// # Safety
/// R is the running engine's current frame (`Engine::regs`), holding the
/// registers the caller names.
unsafe fn widen_registers(e: &mut Engine<'_>, r: *mut Value, width: usize) -> Option<*mut Value> {
    // SAFETY: R as the function's contract (`# Safety`).
    unsafe {
        let width = width.max(e.vm.register_width);
        let frames = (e.frames.len() + 3).max(MIN_FRAMES).checked_next_power_of_two()?;
        let size = width.checked_mul(frames)?.checked_add(e.vm.spread_reserve)?;
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
/// Macro expansion runs Lisp, so this engine is suspended, its collections
/// deferred (`lend::deferred`): callers may keep the callee and argument
/// indices across this. Returns
/// the rebased caller registers, or None with the expansion's own condition
/// pending (GNU signals it when the call reaches the body).
#[cold]
#[inline(never)]
fn force_lazy(e: &mut Engine<'_>, r: *mut Value, function: usize) -> Option<*mut Value> {
    {
        e.vm.execution_fatal = false;
        e.vm.propagated_escape = None;
        e.vm.propagated_eval_depth = None;
        let saved_depth = e.vm.eval_depth;
        e.vm.eval_depth = e.depth_base + e.frames.len();
        debug_assert_eq!(r, e.regs, "a call runs in the current frame");
        let (result, fits) = lend::deferred(e, Nested::Lazy(function));
        e.vm.eval_depth = saved_depth;
        e.fatal_image_failure |= e.vm.execution_fatal;
        if !fits {
            fail(e, "register storage overflow");
            return None;
        }
        match result {
            Ok(_) => Some(e.regs),
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
/// eval.c `Fautoload_do_load (FUNDEF, FUNCTION, Qnil)` for a call site, then
/// the call retried from FUNCTION (`goto retry'): FUNCTION and FUNDEF rooted
/// in the frame across the load.
#[inline(never)]
fn autoload_call<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, retry: Box<(Pc<'x>, RootFrame, (usize, usize), bool)>) -> Continue<'x> {
    let loaded = nested_then(&mut m, gc.by_ref(), Nested::Autoload(&retry.1), (), |_, (), _| {});
    retried(&mut m, loaded, retry)
}
/// The call RETRY names, when the run before it succeeded; else its frame
/// released and the run's escape pending.
fn retried<'x, 'a, 'b>(m: &mut Orch<'_, 'b, 'a>, ok: bool, retry: Box<(Pc<'x>, RootFrame, (usize, usize), bool)>) -> Continue<'x> {
    if ok {
        return Continue::Retry(retry);
    }
    let (_, frame, _, _) = *retry;
    m.leaf(frame, |e, frame| e.vm.drop_frame(frame));
    Continue::Stop
}
/// eval.c funcall_lambda of a `(lambda ARGS . BODY)' list: GNU interprets
/// it with dynamic binding (lexenv nil). VM2 evaluates the list in that
/// dialect, as `(function LIST)', and calls the closure, compiled once
/// while the list is unchanged (`Vm::lambda_list_closure'); the list itself
/// stays the function. The list is rooted in the frame, then its closure.
#[inline(never)]
fn lambda_call<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, retry: Box<(Pc<'x>, RootFrame, (usize, usize), bool)>) -> Continue<'x> {
    // Compiling the list is Romacs's own work, which GNU's interpreter does
    // not do: `debug-on-next-call' waits for the lambda's activation.
    m.leaf((), |e, ()| e.vm.profiler.held += 1);
    let compiled = nested_then(&mut m, gc.by_ref(), Nested::LambdaList(&retry.1), &retry.1, |e, frame, closure| e.vm.set_rooted(frame, 0, closure));
    m.leaf((), |e, ()| e.vm.profiler.held -= 1);
    retried(&mut m, compiled, retry)
}
/// specbind of SLOT to VALUE from compiled code or a call site: a protected
/// variable (most-positive-fixnum, enable-multibyte-characters, ...) is
/// setting-constant, else the binding is made; on failure a watcher's own
/// signal or a forwarded store fault is left pending. A trapped variable's
/// watchers may collect (`watched_write`), so VALUE is rooted across them
/// for the fault's data.
/// The value of SLOT's variable, void signalling as GNU's eval_sub does.
fn global_value(e: &mut Engine<'_>, slot: usize) -> Result<Value> {
    let v = if slot == e.vm.multibyte_slot {
        if e.vm.buffer_text_p() { Value::TRUE } else { Value::NIL }
    } else {
        e.vm.variable_value(slot)?
    };
    if v == Value::UNBOUND {
        raise(e, "void-variable", &[Value::reference(slot + 1, 7)]);
        return Err("unbound variable".into());
    }
    Ok(v)
}

/// Set SLOT's variable to register REG of R when no watcher can run; None,
/// with nothing done, when the variable is watched and the write is a lend
/// (`set_global_lent`).
///
/// # Safety
/// R must point at a live frame holding REG.
unsafe fn set_global_now(e: &mut Engine<'_>, slot: usize, r: *mut Value, reg: usize) -> Option<Result<()>> {
    // SAFETY: R as the function's contract (`# Safety`).
    unsafe {
        if e.vm.protected_variable_slot(slot) {
            raise(e, "setting-constant", &[Value::reference(slot + 1, 7)]);
            return Some(Err("setting constant".into()));
        }
        if e.vm.variable_trapped(e.vm.indirect_variable(slot)) {
            return None;
        }
        let outcome = e.vm.set_variable_value(slot, *r.add(reg));
        Some(write_failed(e, outcome, *r.add(reg)))
    }
}
/// A watched set of SLOT to register REG of the current frame, read again
/// after the watchers.
fn set_global_lent<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, slot: usize, reg: usize) -> Result<()> {
    lend::watched(m, gc, Write::Set(slot, reg), reg, |e, reg, outcome| {
        let value = e.reg(reg);
        write_failed(e, outcome, value)
    })
}
/// A variable write's failure: a variable watcher's own signal, else a
/// forwarded store fault on VALUE, pending.
fn write_failed(e: &mut Engine<'_>, outcome: Result<()>, value: Value) -> Result<()> {
    outcome.inspect_err(|error| {
        if let Some(escape) = e.vm.propagated_escape.take() {
            e.escape = Some(escape);
        } else if let Some((condition, data)) = e.vm.forwarded_condition(error, value) {
            e.escape = Some(Escape::Signal(condition, data));
        }
    })
}

/// A late binding's value: its cell's, or its register's where the
/// binding was unboxed (no closure shares it).
///
/// # Safety
/// R must point at a live frame holding REG.
#[inline(always)]
unsafe fn late_get(e: &Engine<'_>, r: *mut Value, reg: usize) -> Value {
    // SAFETY: R as the function's contract (`# Safety`).
    unsafe {
        let v = *r.add(reg);
        if v.tag() == 5 { e.vm.read_capture_cell(v.payload() as usize) } else { v }
    }
}
/// `late_get` in the current frame, bounds-checked: for orchestration.
fn late_value(e: &Engine<'_>, reg: usize) -> Value {
    let v = e.reg(reg);
    if v.tag() == 5 { e.vm.read_capture_cell(v.payload() as usize) } else { v }
}
/// `late_set` in the current frame, bounds-checked: for orchestration.
fn late_store(e: &mut Engine<'_>, reg: usize, value: Value) {
    let v = e.reg(reg);
    if v.tag() == 5 { e.vm.write_capture_cell(v.payload() as usize, value) } else { e.set_reg(reg, value) }
}
/// Set a late binding's cell, or its register where it was unboxed.
///
/// # Safety
/// R must point at a live frame holding REG.
#[inline(always)]
unsafe fn late_set(e: &mut Engine<'_>, r: *mut Value, reg: usize, value: Value) {
    // SAFETY: R as the function's contract (`# Safety`).
    unsafe {
        let v = *r.add(reg);
        if v.tag() == 5 { e.vm.write_capture_cell(v.payload() as usize, value) } else { *r.add(reg) = value }
    }
}

/// specbind of SLOT to VALUE from compiled code or a call site when no
/// watcher can run: a protected variable (most-positive-fixnum,
/// enable-multibyte-characters, ...) is setting-constant, else the binding
/// is made; on failure a forwarded store fault is left pending. None, with
/// nothing done, when the variable is watched (`bind_special`).
fn bind_special_now(e: &mut Engine<'_>, slot: usize, value: Value) -> Option<Result<()>> {
    if e.vm.protected_variable_slot(slot) {
        raise(e, "setting-constant", &[Value::reference(slot + 1, 7)]);
        return Some(Err("setting constant".into()));
    }
    if e.vm.variable_trapped(e.vm.indirect_variable(slot)) {
        return None;
    }
    let outcome = e.vm.bind_variable(slot, value);
    Some(write_failed(e, outcome, value))
}
/// specbind of SLOT to the value rooted in FRAME at I, from orchestration:
/// a watched variable's watchers may collect, and the value is read back
/// for a fault's data.
fn bind_special<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, slot: usize, frame: &RootFrame, i: usize) -> Result<()> {
    let now = m.leaf((slot, frame, i), |e, (slot, frame, i)| {
        let value = e.vm.rooted(frame, i);
        bind_special_now(e, slot, value)
    });
    now.unwrap_or_else(|| lend::watched(m, gc, Write::Bind(slot, frame, i), (frame, i), |e, (frame, i), outcome| {
        let value = e.vm.rooted(frame, i);
        write_failed(e, outcome, value)
    }))
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
/// so their variable watchers see the let and the unlet. The frame roots the
/// condition's DATA values, then each parameter and argument, read back
/// per binding while watchers run Lisp.
#[inline(never)]
#[expect(clippy::boxed_local, reason = "an orchestration's input fits two words (`orchestrate`), so its caller keeps its sibling calls")]
fn lambda_list_raise<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, failure: Box<(RootFrame, usize, usize, &'static str)>) -> Continue<'x> {
    let (frame, data, bindings, condition) = *failure;
    for k in 0..bindings {
        let at = data + 2 * k;
        let slot = m.leaf((&frame, at), |e, (frame, at)| e.vm.rooted(frame, at).index_if(7).map_or(0, |i| i - 1));
        if let Err(error) = bind_special(&mut m, gc.by_ref(), slot, &frame, at + 1) {
            m.leaf((frame, error), |e, (frame, error)| {
                e.vm.drop_frame(frame);
                if e.escape.is_none() { e.error = Some(error) }
            });
            return Continue::Stop;
        }
    }
    m.leaf((frame, data, condition), |e, (frame, data, condition)| {
        let values = e.vm.rooted_values(&frame)[..data].to_vec();
        e.vm.drop_frame(frame);
        raise(e, condition, &values);
    });
    Continue::Stop
}
/// A failed lambda list walk's values, rooted for `lambda_list_raise`.
fn lambda_list_frame(vm: &mut Vm, (bindings, condition, data): LambdaListFailure) -> Box<(RootFrame, usize, usize, &'static str)> {
    let values: Vec<Value> = data.iter().copied().chain(bindings.iter().flat_map(|&(param, arg)| [param, arg])).collect();
    Box::new((vm.root_frame(&values), data.len(), bindings.len(), condition))
}
/// GNU `(lambda ...)' list functions: a list headed by `lambda' whose cdr
/// is a cons (funcall_lambda signals `invalid-function' otherwise).
fn lambda_list(e: &mut Engine<'_>, target: Value) -> bool {
    target.tag() == 3 && {
        let lambda = Value::reference(e.vm.slot("lambda") + 1, 7);
        e.vm.cons_pair(target).is_ok_and(|(head, tail)| head == lambda && tail.tag() == 3)
    }
}
/// Run CALL, which may load and compile Lisp, from a call site with this
/// engine suspended: its depth and propagation set around it, its escape
/// carried back. THEN gets its value. False with its condition pending, or
/// with the registers out of room.
fn nested_then<'a, 'b, B: Plain>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, call: Nested<'_>, b: B,
    then: impl FnOnce(&mut Engine<'a>, B, Value) + Copy + 'static) -> bool {
    let saved_depth = m.leaf((), |e, ()| {
        e.vm.execution_fatal = false;
        e.vm.propagated_escape = None;
        e.vm.propagated_eval_depth = None;
        e.vm.loader.propagate += 1;
        std::mem::replace(&mut e.vm.eval_depth, e.depth_base + e.frames.len())
    });
    lend::nested(m, gc, call, (b, saved_depth), move |e, (b, saved_depth), result, fits| {
        e.vm.loader.propagate -= 1;
        e.vm.eval_depth = saved_depth;
        e.fatal_image_failure |= e.vm.execution_fatal;
        if !fits {
            fail(e, "register storage overflow");
            return false;
        }
        match result {
            Ok(value) => {
                then(e, b, value);
                true
            }
            Err(error) => {
                carry_escape(e, error);
                false
            }
        }
    })
}
/// A nested run's failure, as the call site's: its own escape crosses back
/// unchanged; else its message is this engine's error.
fn carry_escape(e: &mut Engine<'_>, error: String) {
    if let Some(escape) = e.vm.propagated_escape.take() {
        e.escape = Some(escape);
        e.escape_hooked = std::mem::take(&mut e.vm.propagated_escape_hooked);
        e.failed_eval_depth = e.vm.propagated_eval_depth.take();
    } else {
        e.error = Some(error);
    }
}
/// GNU garbage_collect's end in this engine: RUNNER
/// (`romacs--run-finalizers') runs the doomed finalizers and
/// `post-gc-hook' (`Vm::run_aftermath'), borrowing room near the depth
/// limit, as GNU's handlers have. False with an escape pending.
fn aftermath<'a, 'b>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, runner: Imm) -> bool {
    nested_with_room(m, gc, Nested::Aftermath(runner), (), |_, (), _| {})
}
/// An aftermath runner as plain data: it is a symbol of the initial obarray.
fn plain_runner(vm: &Vm, runner: Option<Value>) -> Option<Imm> {
    runner.map(|runner| Imm::new(vm, runner).expect("the aftermath runner is an interned symbol"))
}
/// `garbage-collect' (`Step::Collect'): collect now; its statistics, or nil
/// when collection is held (GNU's garbage_collection_inhibited), in the
/// call's register; then the aftermath, at once (`Vm::finalizer_runner').
#[inline(never)]
fn collect_explicit<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, pc: Pc<'x>) -> Continue<'x> {
    let explicit = m.leaf((), |e, ()| std::mem::replace(&mut e.vm.collection_log.explicit, true));
    let collected = lend::collect(&mut m, gc.by_ref(), false);
    let runner = m.leaf((pc.a(), collected, explicit), |e, (dest, collected, explicit)| {
        if !collected {
            e.vm.collection_log.explicit = explicit;
            e.set_reg(dest, Value::NIL);
            return None;
        }
        let statistics = e.vm.garbage_collect_statistics();
        e.set_reg(dest, statistics);
        let runner = e.vm.finalizer_runner();
        plain_runner(e.vm, runner)
    });
    collected_then(&mut m, gc, pc, runner)
}
/// `romacs--collect-idle' (`Step::CollectIdle'): collect unless a key press
/// comes first; whether it did, in the call's register; then the aftermath
/// through the gate (a wait for input).
#[inline(never)]
fn collect_idle<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, pc: Pc<'x>) -> Continue<'x> {
    let collected = lend::collect(&mut m, gc.by_ref(), true);
    let runner = m.leaf((pc.a(), collected), |e, (dest, collected)| {
        e.set_reg(dest, if collected { Value::TRUE } else { Value::NIL });
        let runner = if collected { e.safe_runner() } else { None };
        plain_runner(e.vm, runner)
    });
    collected_then(&mut m, gc, pc, runner)
}
/// After a collection a call asked for: RUNNER's aftermath, then the
/// instruction after PC.
fn collected_then<'x, 'a, 'b>(m: &mut Orch<'_, 'b, 'a>, gc: Gc<'_, 'b>, pc: Pc<'x>, runner: Option<Imm>) -> Continue<'x> {
    after_if(runner.is_none_or(|runner| aftermath(m, gc, runner)), pc)
}
/// The instruction after PC when OK, else the pending escape's way out.
fn after_if(ok: bool, pc: Pc<'_>) -> Continue<'_> {
    if ok { Continue::After(pc) } else { Continue::Stop }
}
impl Vm {
    /// One run of a collection's aftermath: RUNNER (`romacs--run-finalizers')
    /// through the trampoline, holding aftermath until it returns, so that an
    /// automatic collection inside it defers to its loop or a later safe
    /// point, never nesting. With
    /// PROPAGATE its escape is the caller's.
    pub(super) fn run_aftermath(&mut self, runner: Value, propagate: bool) -> Result<Value> {
        let trampoline = self.reader_trampoline(0)?;
        self.aftermath_runs += 1;
        self.propagate_next = propagate;
        let outcome = execute_args(self, trampoline, &[runner]);
        self.propagate_next = false;
        self.aftermath_runs -= 1;
        outcome
    }
}
// eval, load, require, macroexpand, autoload-do-load and the host read run
// Lisp with this frame suspended (`call_suspended`). The work, and the copy of
// ARGS it owns, live in `reentrant_step`, so this handler keeps its sibling-call
// dispatch: a frame kept here per call grew the native stack with every `eval`
// a Lisp loop made (eval-buffer's readevalloop, one per top-level form).
/// eval, load, require, macroexpand, autoload-do-load, the host read and the
/// mode line format run Lisp with this frame suspended. The arguments are
/// rooted copies (FRAME), the value goes to the call's register.
#[inline(never)]
fn loader_body<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, (pc, call): (Pc<'x>, Box<(Primitive, usize, usize)>)) -> Continue<'x> {
    let (p, start, argc) = *call;
    let (frame, saved_depth) = m.leaf((start, argc), |e, (start, argc)| {
        let args = e.vm.stack[start..start + argc].to_vec();
        let frame = e.vm.root_frame(&args);
        e.vm.execution_fatal = false;
        // Only an escape produced by THIS operation may be re-raised below.
        e.vm.propagated_escape = None;
        e.vm.propagated_eval_depth = None;
        e.vm.loader.propagate += 1;
        (frame, std::mem::replace(&mut e.vm.eval_depth, e.depth_base + e.frames.len()))
    });
    let ok = lend::nested(&mut m, gc.by_ref(), Nested::Loader(p, &frame), (pc.a(), p, (start, argc), saved_depth),
        |e, (dest, p, (start, argc), saved_depth), result, fits| {
        e.vm.loader.propagate -= 1;
        e.vm.eval_depth = saved_depth;
        e.fatal_image_failure |= e.vm.execution_fatal;
        if !fits {
            fail(e, "register storage overflow");
            return false;
        }
        match result {
            Ok(value) => {
                leave_native(e);
                e.set_reg(dest, value);
                true
            }
            Err(error) => {
                // An `eval` form's own signal or throw crosses back unchanged;
                // nothing has allocated since its engine stored it.
                if let Some(escape) = e.vm.propagated_escape.take() {
                    e.escape = Some(escape);
                    e.escape_hooked = std::mem::take(&mut e.vm.propagated_escape_hooked);
                    e.failed_eval_depth = e.vm.propagated_eval_depth.take();
                    return false;
                }
                if error == "native stack exhausted while compiling Lisp" {
                    native_stack_failure(e, &error);
                    return false;
                }
                if let Some((condition, data)) = e.vm.primitive_condition(p, start, argc, &error) {
                    e.escape = Some(Escape::Signal(condition, data));
                }
                e.error = Some(error);
                false
            }
        }
    });
    m.leaf(frame, |e, frame| e.vm.drop_frame(frame));
    after_if(ok, pc)
}

/// A kernel's case with an operand that is no fixnum: `Vm::float_kernel',
/// else the primitive. A handler of its own, so a kernel's fixnum path needs
/// no stack frame.
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn not_fixnums<const KERNEL: u8>(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let x = *r.add(i.d);
        let y = if i.e == 2 { *r.add(i.d + 1) } else { Value(5) };
        let value = e.vm.float_kernel(KERNEL, x, y);
        if value == Value::UNBOUND {
            return sealed_call(e, r, pc);
        }
        *r.add(i.a) = value;
        advance(e, r, pc)
    }
}
// Binary by default: the kernel table gives each handler one argument count.
macro_rules! numeric {
    ($name:ident,$kernel:expr,$body:expr) => {
        numeric!($name, $kernel, $body, |r: *mut Value, i: &Inst| *r.add(i.d + 1));
    };
    ($name:ident,$kernel:expr,$body:expr,$second:expr) => {
        // SAFETY: a section of this binary's own text, laid out by the
        // `global_asm!` above; it holds only this handler's code.
        #[cfg_attr(all(target_os = "linux", target_arch = "x86_64"), unsafe(link_section = concat!(".text.vm2.", stringify!($name))))]
        /// # Safety
        /// R is the running engine's current frame (`Engine::regs`) and PC an
        /// instruction of the function running in it: the registers and constants
        /// its operands name lie in that frame and code, as lowering checked.
        unsafe fn $name(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            // SAFETY: R and PC as the function's contract (`# Safety`).
            unsafe {
                let i = &*pc;
                let x = *r.add(i.d);
                let y: Value = ($second)(r, i);
                // Both tags at once: a fixnum less one has clear low bits.
                if (x.0.wrapping_sub(1) | y.0.wrapping_sub(1)) & 3 != 0 {
                    return not_fixnums::<{ $kernel }>(e, r, pc);
                }
                let result: Option<Value> = ($body)(x, y);
                match result {
                    Some(v) => {
                        *r.add(i.a) = v;
                        advance(e, r, pc)
                    }
                    None => sealed_call(e, r, pc),
                }
            }
        }
    };
}
numeric!(mul, numbers::op::MUL, |x: Value, y: Value| ((x.0 as i64) >> 2)
    .checked_mul((y.0 as i64) >> 2)
    .and_then(Value::fixnum));
numeric!(add, numbers::op::ADD, |x: Value, y: Value| (x.0 as i64)
    .checked_add((y.0 - 1) as i64)
    .map(|n| Value(n as u64)));
numeric!(sub, numbers::op::SUB, |x: Value, y: Value| (x.0 as i64)
    .checked_sub((y.0 - 1) as i64)
    .map(|n| Value(n as u64)));
numeric!(inc, numbers::op::ADD, |x: Value, _y: Value| (x.0 as i64)
    .checked_add(4)
    .map(|n| Value(n as u64)), |_, _| Value(5));
numeric!(dec, numbers::op::SUB, |x: Value, _y: Value| (x.0 as i64)
    .checked_sub(4)
    .map(|n| Value(n as u64)), |_, _| Value(5));
fn boolean(holds: bool) -> Option<Value> {
    Some(if holds { Value::TRUE } else { Value::NIL })
}
numeric!(less, numbers::op::LESS, |x: Value, y: Value| boolean((x.0 as i64) < (y.0 as i64)));
numeric!(greater, numbers::op::GREATER, |x: Value, y: Value| boolean((x.0 as i64) > (y.0 as i64)));
numeric!(less_equal, numbers::op::LESS_EQUAL, |x: Value, y: Value| boolean((x.0 as i64) <= (y.0 as i64)));
numeric!(greater_equal, numbers::op::GREATER_EQUAL, |x: Value, y: Value| boolean((x.0 as i64) >= (y.0 as i64)));
numeric!(equal, numbers::op::EQUAL, |x: Value, y: Value| boolean(x == y));
// A zero divisor and the one quotient past the fixnums are the primitive's.
numeric!(divide, numbers::op::DIV, |x: Value, y: Value| ((x.0 as i64) >> 2)
    .checked_div((y.0 as i64) >> 2)
    .and_then(Value::fixnum));
/// A kernel whose work is one `Vm' call of its operands, UNBOUND leaving the
/// call to the primitive.
macro_rules! through {
    ($name:ident, |$vm:ident, $x:ident, $y:ident| $body:expr) => {
        /// # Safety
        /// R is the running engine's current frame (`Engine::regs`) and PC an
        /// instruction of the function running in it: the registers and constants
        /// its operands name lie in that frame and code, as lowering checked.
        unsafe fn $name(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            // SAFETY: R and PC as the function's contract (`# Safety`).
            unsafe {
                let i = &*pc;
                let ($x, $y) = (*r.add(i.d), *r.add(i.d + 1));
                let $vm = &mut *e.vm;
                let value = $body;
                if value == Value::UNBOUND {
                    return sealed_call(e, r, pc);
                }
                *r.add(i.a) = value;
                advance(e, r, pc)
            }
        }
    };
}
through!(deep_equal, |vm, x, y| vm.equal_kernel(x, y));
through!(member, |vm, x, y| vm.member_kernel(x, y));
through!(get, |vm, x, y| vm.get_kernel(x, y));
through!(nthcdr, |vm, x, y| vm.nthcdr_kernel(x, y));
through!(max, |_vm, x, y| if x.0 & 3 != 1 || y.0 & 3 != 1 { Value::UNBOUND } else if (y.0 as i64) > (x.0 as i64) { y } else { x });
through!(min, |_vm, x, y| if x.0 & 3 != 1 || y.0 & 3 != 1 { Value::UNBOUND } else if (y.0 as i64) < (x.0 as i64) { y } else { x });
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn aset(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let value = *r.add(i.d + 2);
        if !e.vm.aset_kernel(*r.add(i.d), *r.add(i.d + 1), value) {
            return sealed_call(e, r, pc);
        }
        *r.add(i.a) = value;
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn length(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let x = *r.add(i.d);
        if x != Value::NIL && x.0 & 7 != 3 {
            return sequence_read(e, r, pc);
        }
        let value = e.vm.list_length_kernel(x);
        if value == Value::UNBOUND {
            return sealed_call(e, r, pc);
        }
        *r.add(i.a) = value;
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn list(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let mut tail = Value::NIL;
        for at in (0..i.e).rev() {
            tail = e.vm.alloc_cons(*r.add(i.d + at), tail);
        }
        *r.add(i.a) = tail;
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn insert_string(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        match quiet_insert(e.vm, *r.add(i.d)) {
            None => sealed_call(e, r, pc),
            Some(false) => fail(&mut e, "an insertion's undo record or deactivate-mark failed"),
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
    // An edit of shared text moves the other buffers' positions: the
    // general path does that.
    let (data::Object::String(chars), data::Object::Buffer(text)) = (source, target) else {
        return None;
    };
    if text.base != Value::NIL || !text.indirect.is_empty() {
        return None;
    }
    let length = chars.len();
    if !chars.is_utf8() || !chars.properties.is_empty() || !text.append_utf8(chars.as_str()) {
        return None;
    }
    if vm.quiet_inserted(quiet, length).is_err() {
        return Some(false);
    }
    vm.display_state.version += 1;
    Some(true)
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn remainder(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let x = *r.add(i.d);
        let y = *r.add(i.d + 1);
        if x.tag() != 1 || y.tag() != 1 || y.0 as i64 >> 2 == 0 {
            return sealed_call(e, r, pc);
        }
        *r.add(i.a) = Value::fixnum((x.0 as i64 >> 2) % (y.0 as i64 >> 2)).unwrap();
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn sequence_read(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let x = *r.add(i.d);
        if x == Value::NIL || x.tag() != 0 {
            return sealed_call(e, r, pc);
        }
        let Some(object) = e.vm.objects.get(x.payload() as usize - 1) else {
            return sealed_call(e, r, pc);
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
                return sealed_call(e, r, pc);
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
            return sealed_call(e, r, pc);
        };
        *r.add(i.a) = value;
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn buffer_read(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let x = if i.e == 0 { Value::NIL } else { *r.add(i.d) };
        let Some(value) = buffer_read_value(e.vm, i.value, x) else {
            return sealed_call(e, r, pc);
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
/// A type predicate kernel: BODY maps the argument to a bool with no
/// fallback, as GNU's `Bconsp` and friends do.
macro_rules! predicate {
    ($name:ident, |$vm:ident, $x:ident| $body:expr) => {
        /// # Safety
        /// R is the running engine's current frame (`Engine::regs`) and PC an
        /// instruction of the function running in it: the registers and constants
        /// its operands name lie in that frame and code, as lowering checked.
        unsafe fn $name(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            // SAFETY: R and PC as the function's contract (`# Safety`).
            unsafe {
                let i = &*pc;
                let $x = *r.add(i.d);
                let $vm: &Vm = e.vm;
                *r.add(i.a) = if $body { Value::TRUE } else { Value::NIL };
                advance(e, r, pc)
            }
        }
    };
}
predicate!(consp, |_vm, x| x.0 & 7 == 3);
predicate!(listp, |_vm, x| x == Value::NIL || x.0 & 7 == 3);
predicate!(symbolp, |_vm, x| x == Value::NIL || x == Value::TRUE || x.0 & 7 == 7 && x != Value::UNBOUND);
predicate!(integerp, |vm, x| x.0 & 3 == 1 || matches!(vm.object_if(x), Some(data::Object::Bignum(_))));
predicate!(numberp, |vm, x| x.0 & 3 == 1 || matches!(vm.object_if(x), Some(data::Object::Bignum(_) | data::Object::Float(_))));
predicate!(stringp, |vm, x| is_string(vm, x));
/// Out of line, returning a register-sized value: a wide or address-taken
/// result in the handler keeps a native frame across its dispatch (the
/// bounded-stack regression: `tests/traversal.rs`).
#[inline(never)]
fn is_string(vm: &Vm, x: Value) -> bool {
    vm.str_view(x).is_some()
}
/// `car-safe'/`cdr-safe' of X: the half of a cons, nil for anything else;
/// None when the cons cannot be read here.
#[inline(never)]
fn safe_half(vm: &Vm, x: Value, cdr: bool) -> Option<Value> {
    if x.tag() != 3 {
        return Some(Value::NIL);
    }
    traversal::pair(vm, x).map(|(a, d)| if cdr { d } else { a })
}
macro_rules! safe_half {
    ($name:ident, $cdr:expr) => {
        /// # Safety
        /// R is the running engine's current frame (`Engine::regs`) and PC an
        /// instruction of the function running in it: the registers and constants
        /// its operands name lie in that frame and code, as lowering checked.
        unsafe fn $name(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            // SAFETY: R and PC as the function's contract (`# Safety`).
            unsafe {
                let i = &*pc;
                let Some(value) = safe_half(e.vm, *r.add(i.d), $cdr) else {
                    return sealed_call(e, r, pc);
                };
                *r.add(i.a) = value;
                advance(e, r, pc)
            }
        }
    };
}
safe_half!(car_safe, false);
safe_half!(cdr_safe, true);
/// The function cell of symbol X (nil when void), as `symbol-function'
/// answers; None for anything else, or a macro, whose answer it conses.
#[inline(never)]
fn function_cell(vm: &Vm, x: Value) -> Option<Value> {
    if x == Value::UNBOUND {
        return None;
    }
    let slot = x.index_if(7)?.checked_sub(1)?;
    if vm.macros.contains(&slot) {
        return None;
    }
    let function = vm.slots.get(slot)?.function;
    Some(if function == Value::UNBOUND { Value::NIL } else { function })
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn symbol_function(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let Some(value) = function_cell(e.vm, *r.add(i.d)) else {
            return sealed_call(e, r, pc);
        };
        *r.add(i.a) = value;
        advance(e, r, pc)
    }
}
/// `(nconc LIST TAIL)` where LIST is nil or a private list that ends within
/// the list limit: TAIL on its last cons. Anything else (a non-list, an image
/// cons, a cycle) is the primitive's, which signals as GNU's Fnconc does.
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn nconc2(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let (list, tail) = (*r.add(i.d), *r.add(i.d + 1));
        if list == Value::NIL {
            *r.add(i.a) = tail;
            return advance(e, r, pc);
        }
        let limit = e.vm.list_limit();
        let mut last = list;
        let mut steps = 0;
        loop {
            if last.tag() != 3 || Vm::image_value(last) || steps > limit {
                return sealed_call(e, r, pc);
            }
            let Some(&(_, next)) = e.vm.conses.get(last.payload() as usize) else {
                return sealed_call(e, r, pc);
            };
            if next.tag() != 3 {
                break;
            }
            last = next;
            steps += 1;
        }
        e.vm.conses[last.payload() as usize].1 = tail;
        *r.add(i.a) = list;
        advance(e, r, pc)
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn cons(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        *r.add(i.a) = e.vm.alloc_cons(*r.add(i.d), *r.add(i.d + 1));
        advance(e, r, pc)
    }
}
// Never inlined: its call would give every handler that leaves here a frame.
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn cold(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        match cold_step(&mut e, r, pc) {
            Some(true) => advance(e, r, pc),
            Some(false) => Value::UNBOUND,
            None => {
                debug_assert_eq!(r, e.regs, "an instruction runs in the current frame");
                let lent = match (*pc).op {
                    Op::BindLate(..) => bind_late_lent,
                    Op::UnbindLate(..) => unbind_late_lent,
                    _ => write_late_lent,
                };
                orchestrate(e, Pc::new(pc), lent, continue_at)
            }
        }
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn cell(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let reg = (*pc).a;
        let index = e.vm.alloc_cell(*r.add(reg));
        *r.add(reg) = Value::reference(index, 5);
        advance(e, r, pc)
    }
}
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn make_closure(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let captures = e.vm.functions[e.function].sites[i.c].iter().map(|&reg| *r.add(reg)).collect();
        *r.add(i.a) = e.vm.alloc_closure(Closure { function: i.b, captures, metadata: vec![], document: Value::NIL });
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn closure(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        make_closure(&mut e, r, pc);
        advance(e, r, pc)
    }
}
/// A step's outcome as dispatch wants it: an error or an escape stops.
fn stepped(e: &mut Engine<'_>, outcome: Result<()>) -> bool {
    match outcome {
        Ok(()) => e.escape.is_none(),
        Err(error) => {
            e.error = Some(error);
            false
        }
    }
}
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn assign_global_step(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Option<bool> {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe { set_global_now(e, (*pc).a, r, (*pc).b).map(|outcome| stepped(e, outcome)) }
}
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn bind_step(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Option<bool> {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe { bind_special_now(e, (*pc).a, *r.add((*pc).b)).map(|outcome| stepped(e, outcome)) }
}
#[inline(never)]
/// # Safety
/// PC is an instruction of the function running in the current frame.
unsafe fn unbind_step(e: &mut Engine<'_>, pc: *const Inst) -> Option<bool> {
    // SAFETY: PC as the function's contract.
    unsafe {
        let n = (*pc).a;
        if n > e.vm.dynamic.len() - e.dynamic {
            return Some(stepped(e, Err("invalid image dynamic binding depth".into())));
        }
        let signal = unbind_now(e, n)?;
        if let Some(signal) = signal {
            e.escape = Some(signal);
        }
        Some(e.escape.is_none())
    }
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn assign_global(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        match assign_global_step(&mut e, r, pc) {
            Some(true) => advance(e, r, pc),
            Some(false) => Value::UNBOUND,
            None => orchestrate(e, Pc::new(pc), assign_lent, continue_at),
        }
    }
}
#[inline(never)]
fn assign_lent<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, pc: Pc<'x>) -> Continue<'x> {
    let outcome = set_global_lent(&mut m, gc.by_ref(), pc.a(), pc.b());
    after_if(m.leaf(outcome, |e, outcome| stepped(e, outcome)), pc)
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn bind(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        match bind_step(&mut e, r, pc) {
            Some(true) => advance(e, r, pc),
            Some(false) => Value::UNBOUND,
            None => orchestrate(e, Pc::new(pc), bind_lent, continue_at),
        }
    }
}
#[inline(never)]
fn bind_lent<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, pc: Pc<'x>) -> Continue<'x> {
    let frame = m.leaf(pc.b(), |e, reg| { let value = e.reg(reg); e.vm.root_frame(&[value]) });
    let outcome = bind_special(&mut m, gc.by_ref(), pc.a(), &frame, 0);
    after_if(m.leaf((frame, outcome), |e, (frame, outcome)| {
        e.vm.drop_frame(frame);
        stepped(e, outcome)
    }), pc)
}
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn unbind(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        match unbind_step(&mut e, pc) {
            Some(true) => advance(e, r, pc),
            Some(false) => Value::UNBOUND,
            None => orchestrate(e, Pc::new(pc), unbind_lent_at, continue_at),
        }
    }
}
#[inline(never)]
fn unbind_lent_at<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, pc: Pc<'x>) -> Continue<'x> {
    unbind_lent(&mut m, gc.by_ref(), pc.a());
    after_if(m.leaf((), |e, ()| e.escape.is_none()), pc)
}
// Owned result/error/control payloads must be dropped before tail dispatch.
// Keeping them in `cold` retains one native frame per cold instruction even
// when the Lisp program's activation depth stays constant.
#[cold]
#[inline(never)]
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
unsafe fn cold_step(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Option<bool> {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        // A late binding's write to a watched variable: a lend (`cold_lent`).
        let mut lent = false;
        let outcome = (|| -> Result<()> {
            match (*pc).op {
                Op::Global(reg, slot) => *r.add(reg) = global_value(e, slot)?,
                // A late binding (Op::BindLate): dynamic when its symbol is
                // special now, its cell then forwarding to the dynamic value.
                Op::BindLate(slot, cell) => {
                    if e.vm.slot_is_special_now(slot) {
                        let Some(outcome) = bind_special_now(e, slot, late_get(e, r, cell)) else {
                            lent = true;
                            return Ok(());
                        };
                        outcome?;
                        late_set(e, r, cell, Value::UNBOUND);
                    }
                }
                Op::UnbindLate(cell) => {
                    if late_get(e, r, cell) == Value::UNBOUND {
                        if e.vm.dynamic.len() <= e.dynamic {
                            return Err("invalid late dynamic binding depth".into());
                        }
                        let Some(signal) = unbind_now(e, 1) else {
                            lent = true;
                            return Ok(());
                        };
                        if let Some(signal) = signal {
                            e.escape = Some(signal);
                            return Ok(());
                        }
                    }
                }
                Op::ReadLate(reg, cell, slot) => {
                    let v = late_get(e, r, cell);
                    *r.add(reg) = if v == Value::UNBOUND { global_value(e, slot)? } else { v };
                }
                Op::WriteLate(cell, reg, slot) => {
                    if late_get(e, r, cell) == Value::UNBOUND {
                        let Some(outcome) = set_global_now(e, slot, r, reg) else {
                            lent = true;
                            return Ok(());
                        };
                        outcome?;
                    } else {
                        late_set(e, r, cell, *r.add(reg));
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
                            // The protected form is done: aftermath its cleanup
                            // or body left pending is the next poll's, soon.
                            if e.vm.aftermath_pending() && !e.in_cleanup() && !e.vm.aftermath_held() { e.budget = 0; }
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
                | Op::Cell(..)
                | Op::SetGlobal(..)
                | Op::Bind(..)
                | Op::Unbind(..)
                | Op::Closure(..)
                | Op::ReadCell(..)
                | Op::WriteCell(..)
                | Op::LateForwarded(..)
                | Op::Jump(..)
                | Op::Unless(..)
                | Op::Return(..)
                | Op::Call(..)
                | Op::Direct(..) | Op::Sealed(..) => unreachable!("lowering assigns hot handlers"),
            }
            Ok(())
        })();
        if lent {
            return None;
        }
        Some(stepped(e, outcome))
    }
}
/// Op::BindLate's watched binding: the slot bound to the late binding's
/// cell value, then the cell forwarding to the dynamic value.
#[inline(never)]
fn bind_late_lent<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, pc: Pc<'x>) -> Continue<'x> {
    let (slot, cell) = pc.late_bind();
    let frame = m.leaf(cell, |e, cell| { let value = late_value(e, cell); e.vm.root_frame(&[value]) });
    let outcome = bind_special(&mut m, gc.by_ref(), slot, &frame, 0);
    after_if(m.leaf((frame, outcome, cell), |e, (frame, outcome, cell)| {
        e.vm.drop_frame(frame);
        if outcome.is_ok() {
            late_store(e, cell, Value::UNBOUND);
        }
        stepped(e, outcome)
    }), pc)
}
/// Op::UnbindLate's watched unbinding.
#[inline(never)]
fn unbind_late_lent<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, pc: Pc<'x>) -> Continue<'x> {
    unbind_lent(&mut m, gc.by_ref(), 1);
    after_if(m.leaf((), |e, ()| e.escape.is_none()), pc)
}
/// Op::WriteLate's watched set of its slot.
#[inline(never)]
fn write_late_lent<'x, 'a, 'b>(mut m: Orch<'_, 'b, 'a>, mut gc: Gc<'_, 'b>, pc: Pc<'x>) -> Continue<'x> {
    let (slot, reg) = pc.late_write();
    let outcome = set_global_lent(&mut m, gc.by_ref(), slot, reg);
    after_if(m.leaf(outcome, |e, outcome| stepped(e, outcome)), pc)
}

#[cfg(test)]
thread_local! {
    /// The engine's polls (`poll_resume') on this thread, for tests.
    pub(crate) static POLLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
            let direct = lower(&vm, Op::Direct(0, slot, 0, 1, argc));
            assert!(
                std::ptr::fn_addr_eq(direct.handler, direct_kernel as Handler),
                "{name} still uses generic dispatch"
            );
            if name == "plist-get" {
                continue;
            }
            let sealed = lower(&vm, Op::Sealed(0, slot, 1, argc));
            assert!(std::ptr::fn_addr_eq(direct.kernel, sealed.handler),
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
    fn registers_stay_widened_and_nil_across_runs() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun register-probe (n) (if (> n 0) (1+ (register-probe (1- n))) 0))")
            .unwrap();
        vm.eval_source_raw("(register-probe 100)").unwrap();
        let widened = vm.execution.registers.len();
        assert!(widened > vm.register_width * MIN_FRAMES, "the recursion went past MIN_FRAMES");
        assert!(vm.execution.registers.iter().all(|&value| value == Value::NIL));
        vm.eval_source_raw("(register-probe 1)").unwrap();
        assert_eq!(vm.execution.registers.len(), widened);
        assert!(vm.execution.registers.iter().all(|&value| value == Value::NIL));
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
