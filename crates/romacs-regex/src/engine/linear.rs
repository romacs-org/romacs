//! The linear-time matcher (R92 step 2).
//!
//! A Pike VM over the rewind view: the same bytecode the backtracker runs,
//! stepped one character at a time with every live thread in the order the
//! backtracker would try it, so the first thread to reach `succeed` is the
//! match GNU's backtracker returns -- registers included -- while the work is
//! bounded by `text * pattern`, never exponential.  It runs two ways:
//!
//! * **anchored** ([`run_anchored`]): one candidate start, the budgeted
//!   backtracker's fallback when a candidate turns catastrophic;
//! * **unanchored** ([`run_search`]): a whole forward search in ONE pass.  A
//!   new thread is started at every candidate position with the lowest
//!   priority, so an earlier start always outranks a later one, and a match
//!   cuts every thread of lower priority -- GNU's "first candidate that
//!   matches wins", without trying the candidates one by one.  This is what
//!   removes the quadratic search loop: `re_search` tries each candidate with
//!   an anchored attempt, which costs up to the rest of the text per
//!   candidate even when every attempt is linear.
//!
//! # GNU's empty-loop rule
//!
//! GNU's `on_failure_jump_loop` (regex-emacs.c, `CHECK_INFINITE_LOOP`) leaves
//! a loop whose body matched nothing: when the same loop op already has a
//! failure point at the current position on the stack, it jumps to the exit
//! instead of pushing.  So an empty iteration DOES run once -- its group
//! registers stick, `(string-match "\\(a*\\)*b" "aab")` gives group 1 =
//! (2 2) -- and only the second empty pass is cut.  The failure points at the
//! current position are exactly the choice points the current path took
//! since it last consumed a character, so a thread's closure carries the set
//! of loop ops whose body it entered at this position (`mask`), and a loop op
//! already in the set takes its exit only.  Closure nodes are deduplicated by
//! `(pc, mask)`: two paths at the same pc with the same set have the same
//! future, and the first (higher priority) one is the backtracker's.  After a
//! character is consumed the set is empty again, so consuming threads are
//! deduplicated by pc alone.  Patterns with no loop op on an epsilon cycle
//! need no mask (`Linear::Plain`): every closure is acyclic and the plain
//! per-pc dedup is exact.
//!
//! # What it does not run
//!
//! Backreferences (not a regular language), interval counters, a non-greedy
//! loop over a nullable body (`on_failure_jump_nastyloop`, whose cycle check
//! reads marker frames), `??`, and POSIX-longest patterns: [`Linear`] is
//! `Ineligible` and the backtracker runs them.  Neither does it model GNU's
//! fail-stack limit: an unanchored run that could reach it hands the
//! candidate back ([`SearchOutcome::Defer`]) so the backtracker decides it
//! with GNU's "Stack overflow in regexp matcher".

use crate::engine::matcher::step::{
    SyntaxAssertion, evaluate_syntax_assertion, match_anychar_at, match_categoryspec_at,
    match_charset_at, match_exactn_char_at, match_syntaxspec_at, match_syntaxspecset_at,
    re_text_char,
};
use crate::engine::opcode::{RegexOp, extract_number, extract_number_u16, opcode_len};
use crate::engine::pattern::{CompiledPattern, MatchRegisters};
use crate::engine::syntax::SyntaxLookup;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

/// How the linear matcher may run a compiled pattern.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Linear {
    /// The backtracker only (see the module docs).
    #[default]
    Ineligible,
    /// No loop op on an epsilon cycle: per-pc closure dedup is exact.
    Plain,
    /// These `on_failure_jump_loop` pcs lie on epsilon cycles; bit `i` of a
    /// closure's mask is `loops[i]`.
    PathSensitive(Box<[u32]>),
}

impl Linear {
    #[inline]
    pub fn eligible(&self) -> bool {
        !matches!(self, Linear::Ineligible)
    }

    fn loop_bit(&self, pc: usize) -> Option<u64> {
        match self {
            Linear::PathSensitive(loops) => loops
                .binary_search(&(pc as u32))
                .ok()
                .map(|index| 1u64 << index),
            _ => None,
        }
    }
}

/// Most cyclic loop ops a path-sensitive pattern may have (mask bits).
const MAX_CYCLIC_LOOPS: usize = 64;

/// Classify `rewind` (the rewind view of `pattern`'s bytecode).
pub fn classify(pattern: &CompiledPattern, rewind: &[u8]) -> Linear {
    if pattern.posix || pattern.has_nongreedy_optional {
        return Linear::Ineligible;
    }
    let mut loops: Vec<u32> = Vec::new();
    let mut pc = 0usize;
    while pc < rewind.len() {
        let Some(op) = RegexOp::from_byte(rewind[pc]) else {
            return Linear::Ineligible;
        };
        match op {
            RegexOp::Duplicate
            | RegexOp::SucceedN
            | RegexOp::JumpN
            | RegexOp::SetNumberAt
            | RegexOp::OnFailureJumpNastyloop => return Linear::Ineligible,
            RegexOp::OnFailureJumpLoop if on_epsilon_cycle(rewind, pc, None) => {
                loops.push(pc as u32);
            }
            _ => {}
        }
        let Some(len) = opcode_len(rewind, pc) else {
            return Linear::Ineligible;
        };
        pc += len;
    }
    if loops.is_empty() {
        // Fail closed if some epsilon cycle exists anyway (GNU emits none
        // without a loop op; a hand-assembled buffer might).
        return if any_epsilon_cycle(rewind, &[]) {
            Linear::Ineligible
        } else {
            Linear::Plain
        };
    }
    // Every epsilon cycle must pass a loop op, whose mask bit then bounds the
    // closure; a cycle through none would never terminate in GNU either.
    if loops.len() > MAX_CYCLIC_LOOPS || any_epsilon_cycle(rewind, &loops) {
        return Linear::Ineligible;
    }
    Linear::PathSensitive(loops.into_boxed_slice())
}

/// The non-consuming successors of the op at `pc` (consuming ops, `succeed`
/// and the end of the program have none).
fn epsilon_successors(bytecode: &[u8], pc: usize) -> SmallVec<[usize; 2]> {
    let mut out = SmallVec::new();
    let Some(op) = RegexOp::from_byte(bytecode[pc]) else {
        return out;
    };
    let target = |pc: usize| (pc as i64 + 3 + extract_number(bytecode, pc + 1) as i64) as usize;
    match op {
        RegexOp::NoOp
        | RegexOp::BegLine
        | RegexOp::EndLine
        | RegexOp::BegBuf
        | RegexOp::EndBuf
        | RegexOp::AtDot
        | RegexOp::WordBound
        | RegexOp::NotWordBound
        | RegexOp::WordBeg
        | RegexOp::WordEnd
        | RegexOp::SymBeg
        | RegexOp::SymEnd => out.push(pc + 1),
        RegexOp::StartMemory | RegexOp::StopMemory => out.push(pc + 2),
        RegexOp::Jump => out.push(target(pc)),
        RegexOp::OnFailureJump
        | RegexOp::OnFailureKeepStringJump
        | RegexOp::OnFailureJumpLoop
        | RegexOp::OnFailureJumpNastyloop
        | RegexOp::OnFailureJumpSmart => {
            out.push(pc + 3);
            out.push(target(pc));
        }
        _ => {}
    }
    out
}

/// Whether `start` reaches itself over epsilon edges, never entering a node
/// in `avoid`.
fn on_epsilon_cycle(bytecode: &[u8], start: usize, avoid: Option<&[u32]>) -> bool {
    let mut stack: Vec<usize> = epsilon_successors(bytecode, start).to_vec();
    let mut visited = vec![false; bytecode.len() + 1];
    while let Some(n) = stack.pop() {
        if n == start {
            return true;
        }
        if n >= bytecode.len() || visited[n] {
            continue;
        }
        if avoid.is_some_and(|avoid| avoid.binary_search(&(n as u32)).is_ok()) {
            continue;
        }
        visited[n] = true;
        stack.extend(epsilon_successors(bytecode, n));
    }
    false
}

/// Whether any epsilon cycle avoids every op in `loops` (sorted).
fn any_epsilon_cycle(bytecode: &[u8], loops: &[u32]) -> bool {
    let mut pc = 0usize;
    while pc < bytecode.len() {
        if loops.binary_search(&(pc as u32)).is_err() && on_epsilon_cycle(bytecode, pc, Some(loops))
        {
            return true;
        }
        match opcode_len(bytecode, pc) {
            Some(len) => pc += len,
            None => return true,
        }
    }
    false
}

// ---------------------------------------------------------------------------
// The VM
// ---------------------------------------------------------------------------

/// A thread's capture slots: `start(g)` at `2*g`, `end(g)` at `2*g+1`.
/// Group 0 is filled in from the thread's start and the match end.
type Caps = SmallVec<[Option<u32>; 8]>;

/// `pc` of a thread that has reached the end of the program.
const MATCH_PC: usize = usize::MAX;

#[derive(Clone)]
struct Thread {
    /// The consuming op the thread waits on, or [`MATCH_PC`].
    pc: usize,
    /// Bytes of an `exactn` literal already consumed.
    lit_off: usize,
    /// The candidate position this thread's match would start at.
    start: usize,
    caps: Caps,
}

/// Reused per-thread VM state.  Dedup stamps are generation-numbered, so
/// nothing is cleared between steps or runs.
#[derive(Default)]
struct Scratch {
    seen: Vec<u32>,
    masks: FxHashMap<(u32, u64), u32>,
    generation: u32,
    clist: Vec<Thread>,
    nlist: Vec<Thread>,
    stack: Vec<(usize, u64, Caps)>,
}

thread_local! {
    static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::new(Scratch::default());
}

impl Scratch {
    fn next_generation(&mut self, len: usize) -> u32 {
        if self.seen.len() < len + 1 {
            self.seen.resize(len + 1, 0);
        }
        if self.generation == u32::MAX {
            self.seen.iter_mut().for_each(|x| *x = 0);
            self.masks.clear();
            self.generation = 0;
        }
        self.generation += 1;
        self.generation
    }
}

/// Everything the closure and the step read, fixed for one run.
struct Vm<'a> {
    pattern: &'a CompiledPattern,
    bytecode: &'a [u8],
    linear: &'a Linear,
    text: &'a [u8],
    stop: usize,
    point: usize,
    syntax: &'a dyn SyntaxLookup,
    num_regs: usize,
}

impl Vm<'_> {
    /// Append the closure of `start_pc` at position `d` to `list`: every
    /// consuming op (and `succeed`) reachable without consuming, in the
    /// backtracker's try order (fall-through before jump target).
    fn add_thread(
        &self,
        scratch: &mut Scratch,
        list: &mut Vec<Thread>,
        start_pc: usize,
        start: usize,
        caps: Caps,
        d: usize,
    ) {
        let generation = scratch.generation;
        let path_sensitive = matches!(self.linear, Linear::PathSensitive(_));
        let bytecode = self.bytecode;
        scratch.stack.clear();
        scratch.stack.push((start_pc, 0, caps));
        while let Some((pc, mask, caps)) = scratch.stack.pop() {
            let consuming = pc >= bytecode.len() || is_consuming(bytecode[pc]);
            if consuming || !path_sensitive {
                let slot = pc.min(bytecode.len());
                if scratch.seen[slot] == generation {
                    continue;
                }
                scratch.seen[slot] = generation;
            } else {
                let stamp = scratch.masks.entry((pc as u32, mask)).or_insert(0);
                if *stamp == generation {
                    continue;
                }
                *stamp = generation;
            }
            if pc >= bytecode.len() {
                list.push(Thread {
                    pc: MATCH_PC,
                    lit_off: 0,
                    start,
                    caps,
                });
                continue;
            }
            let Some(op) = RegexOp::from_byte(bytecode[pc]) else {
                continue;
            };
            let holds = |cond: bool, stack: &mut Vec<(usize, u64, Caps)>, caps: Caps| {
                if cond {
                    stack.push((pc + 1, mask, caps));
                }
            };
            match op {
                RegexOp::NoOp => scratch.stack.push((pc + 1, mask, caps)),
                RegexOp::Succeed | RegexOp::PosixEnd => list.push(Thread {
                    pc: MATCH_PC,
                    lit_off: 0,
                    start,
                    caps,
                }),
                RegexOp::StartMemory | RegexOp::StopMemory => {
                    let group = bytecode[pc + 1] as usize;
                    let mut caps = caps;
                    if group < self.num_regs {
                        let slot = 2 * group + usize::from(op == RegexOp::StopMemory);
                        caps[slot] = Some(d as u32);
                    }
                    scratch.stack.push((pc + 2, mask, caps));
                }
                RegexOp::BegLine => holds(
                    d == 0 || self.text[d - 1] == b'\n',
                    &mut scratch.stack,
                    caps,
                ),
                RegexOp::EndLine => holds(
                    d >= self.text.len() || self.text[d] == b'\n',
                    &mut scratch.stack,
                    caps,
                ),
                RegexOp::BegBuf => holds(d == 0, &mut scratch.stack, caps),
                RegexOp::EndBuf => holds(d == self.text.len(), &mut scratch.stack, caps),
                RegexOp::AtDot => holds(d == self.point, &mut scratch.stack, caps),
                RegexOp::WordBound
                | RegexOp::NotWordBound
                | RegexOp::WordBeg
                | RegexOp::WordEnd
                | RegexOp::SymBeg
                | RegexOp::SymEnd => holds(
                    evaluate_syntax_assertion(
                        SyntaxAssertion::from_regex_op(op),
                        self.text,
                        d,
                        self.stop,
                        self.pattern.target_multibyte,
                        self.syntax,
                    ),
                    &mut scratch.stack,
                    caps,
                ),
                RegexOp::Jump => {
                    let target = (pc as i64 + 3 + extract_number(bytecode, pc + 1) as i64) as usize;
                    scratch.stack.push((target, mask, caps));
                }
                RegexOp::OnFailureJump
                | RegexOp::OnFailureKeepStringJump
                | RegexOp::OnFailureJumpLoop
                | RegexOp::OnFailureJumpSmart => {
                    let target = (pc as i64 + 3 + extract_number(bytecode, pc + 1) as i64) as usize;
                    match self.linear.loop_bit(pc) {
                        // CHECK_INFINITE_LOOP found this loop's failure point
                        // at this position: leave by the jump target only.
                        Some(bit) if mask & bit != 0 => scratch.stack.push((target, mask, caps)),
                        // Push the lower-priority target first so the
                        // fall-through is explored first.
                        bit => {
                            scratch.stack.push((target, mask, caps.clone()));
                            scratch.stack.push((pc + 3, mask | bit.unwrap_or(0), caps));
                        }
                    }
                }
                RegexOp::Exactn
                | RegexOp::AnyChar
                | RegexOp::Charset
                | RegexOp::CharsetNot
                | RegexOp::SyntaxSpec
                | RegexOp::NotSyntaxSpec
                | RegexOp::SyntaxSpecSet
                | RegexOp::CategorySpec
                | RegexOp::NotCategorySpec => list.push(Thread {
                    pc,
                    lit_off: 0,
                    start,
                    caps,
                }),
                // `classify` keeps these patterns off the VM.
                RegexOp::Duplicate
                | RegexOp::SucceedN
                | RegexOp::JumpN
                | RegexOp::SetNumberAt
                | RegexOp::OnFailureJumpNastyloop => {
                    debug_assert!(false, "linear VM reached an ineligible op {op:?}");
                }
            }
        }
    }

    /// Run the consuming op of `thread` at `d`: where it resumes (`pc`,
    /// literal offset) and the position after the character, or `None`.
    fn step(&self, thread: &Thread, d: usize) -> Option<(usize, usize, usize)> {
        let bytecode = self.bytecode;
        let pc = thread.pc;
        let tm = self.pattern.target_multibyte;
        let translate = &self.pattern.translate;
        let (next_pc, len) = match RegexOp::from_byte(bytecode[pc])? {
            RegexOp::Exactn => {
                let count = bytecode[pc + 1] as usize;
                let lit = &bytecode[pc + 2..pc + 2 + count];
                let (pat_advance, text_advance) = match_exactn_char_at(
                    lit,
                    thread.lit_off,
                    self.pattern.multibyte,
                    tm,
                    translate,
                    self.text,
                    d,
                    self.stop,
                )?;
                let lit_off = thread.lit_off + pat_advance;
                if lit_off < count {
                    return Some((pc, lit_off, d + text_advance));
                }
                (pc + 2 + count, text_advance)
            }
            RegexOp::AnyChar => (
                pc + 1,
                match_anychar_at(self.text, d, self.stop, tm, translate)?,
            ),
            RegexOp::Charset | RegexOp::CharsetNot => (
                pc + 2 + (bytecode[pc + 1] as usize & 0x7F),
                match_charset_at(
                    self.pattern,
                    pc,
                    self.text,
                    d,
                    self.stop,
                    tm,
                    translate,
                    self.syntax,
                )?,
            ),
            op @ (RegexOp::SyntaxSpec | RegexOp::NotSyntaxSpec) => (
                pc + 2,
                match_syntaxspec_at(
                    bytecode[pc + 1],
                    op == RegexOp::NotSyntaxSpec,
                    self.text,
                    d,
                    self.stop,
                    tm,
                    self.syntax,
                )?,
            ),
            RegexOp::SyntaxSpecSet => (
                pc + 3,
                match_syntaxspecset_at(
                    extract_number_u16(bytecode, pc + 1),
                    self.text,
                    d,
                    self.stop,
                    tm,
                    self.syntax,
                )?,
            ),
            op @ (RegexOp::CategorySpec | RegexOp::NotCategorySpec) => (
                pc + 2,
                match_categoryspec_at(
                    bytecode[pc + 1],
                    op == RegexOp::NotCategorySpec,
                    self.text,
                    d,
                    self.stop,
                    tm,
                    self.syntax,
                )?,
            ),
            _ => return None,
        };
        Some((next_pc, 0, d + len))
    }
}

#[inline]
fn is_consuming(byte: u8) -> bool {
    matches!(
        RegexOp::from_byte(byte),
        Some(
            RegexOp::Exactn
                | RegexOp::AnyChar
                | RegexOp::Charset
                | RegexOp::CharsetNot
                | RegexOp::SyntaxSpec
                | RegexOp::NotSyntaxSpec
                | RegexOp::SyntaxSpecSet
                | RegexOp::CategorySpec
                | RegexOp::NotCategorySpec
                | RegexOp::Succeed
                | RegexOp::PosixEnd
        )
    )
}

/// The registers of a match from `start` to `end` with `caps`.
fn registers(num_regs: usize, start: usize, end: usize, caps: &Caps) -> MatchRegisters {
    let mut regs = MatchRegisters::new(num_regs);
    regs.start[0] = start as i64;
    regs.end[0] = end as i64;
    for g in 1..num_regs {
        let slot = |i: usize| caps.get(i).copied().flatten().map_or(-1, i64::from);
        regs.start[g] = slot(2 * g);
        regs.end[g] = slot(2 * g + 1);
    }
    regs
}

/// One anchored candidate: the match and the furthest position any thread
/// reached (no backtracker path from `pos` consumes past it).
pub struct AnchoredOutcome {
    pub found: Option<(usize, MatchRegisters)>,
    pub reach: usize,
}

/// The result of an unanchored forward search.
pub enum SearchOutcome {
    /// The match GNU's candidate loop returns: its start and registers.
    Found(usize, MatchRegisters),
    /// No candidate in the range matches.
    NotFound,
    /// Every candidate before this one fails; this one's paths have run far
    /// enough that GNU's fail stack might overflow there, so the backtracker
    /// must decide it.
    Defer(usize),
    /// A quit is pending: abandon the search as the backtracker does.
    Quit,
}

/// Steps between quit polls.
const QUIT_POLL_STEPS: usize = 4096;

fn with_scratch<R>(f: impl FnOnce(&mut Scratch) -> R) -> R {
    SCRATCH.with(|cell| match cell.try_borrow_mut() {
        Ok(mut scratch) => f(&mut scratch),
        // A syntax callback re-entering the matcher gets fresh state.
        Err(_) => f(&mut Scratch::default()),
    })
}

fn vm<'a>(
    pattern: &'a CompiledPattern,
    text: &'a [u8],
    stop: usize,
    syntax: &'a dyn SyntaxLookup,
    point: usize,
) -> Vm<'a> {
    Vm {
        pattern,
        bytecode: pattern
            .rewind_bytecode()
            .expect("a linear-eligible pattern has a rewind view"),
        linear: &pattern.linear,
        text,
        stop,
        point,
        syntax,
        num_regs: pattern.re_nsub + 1,
    }
}

/// Match `pattern` anchored at `pos`, ending by `stop`.
pub fn run_anchored(
    pattern: &CompiledPattern,
    text: &[u8],
    pos: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
) -> AnchoredOutcome {
    debug_assert!(pattern.linear.eligible());
    let vm = vm(pattern, text, stop, syntax, point);
    with_scratch(|scratch| {
        let mut clist = std::mem::take(&mut scratch.clist);
        let mut nlist = std::mem::take(&mut scratch.nlist);
        clist.clear();
        let mut matched: Option<(usize, Caps)> = None;
        let mut d = pos;
        scratch.next_generation(vm.bytecode.len());
        let caps: Caps = smallvec::smallvec![None; 2 * vm.num_regs];
        vm.add_thread(scratch, &mut clist, 0, pos, caps, d);
        while !clist.is_empty() {
            let cur_len = if d < stop {
                re_text_char(text, d, pattern.target_multibyte).map(|(_, l)| l)
            } else {
                None
            };
            scratch.next_generation(vm.bytecode.len());
            nlist.clear();
            for thread in &clist {
                if thread.pc == MATCH_PC {
                    // A thread past `stop` (a multibyte character straddling
                    // it) fails the backtracker's trailing check.
                    if d <= stop {
                        matched = Some((d, thread.caps.clone()));
                        break;
                    }
                    continue;
                }
                advance(&vm, scratch, &mut nlist, thread, d);
            }
            std::mem::swap(&mut clist, &mut nlist);
            match cur_len {
                Some(len) if !clist.is_empty() => d += len,
                _ => break,
            }
        }
        scratch.clist = clist;
        scratch.nlist = nlist;
        AnchoredOutcome {
            found: matched.map(|(end, caps)| (end, registers(vm.num_regs, pos, end, &caps))),
            reach: d,
        }
    })
}

/// Step `thread` over the character at `d`, appending its successors.
#[inline]
fn advance(vm: &Vm<'_>, scratch: &mut Scratch, nlist: &mut Vec<Thread>, thread: &Thread, d: usize) {
    if let Some((next_pc, lit_off, next_d)) = vm.step(thread, d) {
        if lit_off != 0 {
            // Mid-literal: no closure until the literal is done.
            nlist.push(Thread {
                pc: next_pc,
                lit_off,
                start: thread.start,
                caps: thread.caps.clone(),
            });
        } else {
            vm.add_thread(
                scratch,
                nlist,
                next_pc,
                thread.start,
                thread.caps.clone(),
                next_d,
            );
        }
    }
}

/// The forward search `re_search` would run over candidates `from..=last`
/// (each stopping by `stop`), in one pass.
///
/// `skip`: the fastmap, when it may reject a position whose byte cannot
/// start a match (an accurate map, no case translation, not nullable); used
/// only while no thread is alive.  `overflow_span`: hand a candidate back
/// once some thread has consumed this many bytes since its start (see
/// [`SearchOutcome::Defer`]); `usize::MAX` never does.
#[allow(clippy::too_many_arguments)]
pub fn run_search(
    pattern: &CompiledPattern,
    text: &[u8],
    from: usize,
    last: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
    skip: Option<&[bool; 256]>,
    overflow_span: usize,
) -> SearchOutcome {
    debug_assert!(pattern.linear.eligible());
    let vm = vm(pattern, text, stop, syntax, point);
    let multibyte = pattern.target_multibyte;
    let text_len = text.len();
    let last = last.min(text_len);
    with_scratch(|scratch| {
        let mut clist = std::mem::take(&mut scratch.clist);
        let mut nlist = std::mem::take(&mut scratch.nlist);
        clist.clear();
        let mut matched: Option<(usize, usize, Caps)> = None;
        let mut d = from;
        let mut steps = 0usize;
        let empty_caps: Caps = smallvec::smallvec![None; 2 * vm.num_regs];
        scratch.next_generation(vm.bytecode.len());
        let outcome = loop {
            // A new candidate at `d`, below every live thread in priority.
            if matched.is_none() && d <= last {
                if clist.is_empty() {
                    // Nothing alive: skip to the next position the fastmap
                    // and the character boundaries admit.
                    while d < text_len
                        && d <= last
                        && ((multibyte && (text[d] & 0xC0) == 0x80)
                            || skip.is_some_and(|map| !map[text[d] as usize]))
                    {
                        d += 1;
                    }
                    if d > last {
                        break SearchOutcome::NotFound;
                    }
                    scratch.next_generation(vm.bytecode.len());
                }
                vm.add_thread(scratch, &mut clist, 0, d, empty_caps.clone(), d);
            }
            if clist.is_empty() {
                if matched.is_some() || d >= last.min(stop) {
                    break match matched.take() {
                        Some((start, end, caps)) => {
                            SearchOutcome::Found(start, registers(vm.num_regs, start, end, &caps))
                        }
                        None => SearchOutcome::NotFound,
                    };
                }
            } else if d - clist[0].start >= overflow_span {
                // The oldest live candidate: every earlier one has failed.
                break SearchOutcome::Defer(clist[0].start);
            }
            steps += 1;
            if steps.is_multiple_of(QUIT_POLL_STEPS) && crate::host::quit_pending() {
                break SearchOutcome::Quit;
            }
            let cur_len = if d < stop.min(text_len) {
                re_text_char(text, d, multibyte).map(|(_, l)| l)
            } else {
                None
            };
            scratch.next_generation(vm.bytecode.len());
            nlist.clear();
            for thread in &clist {
                if thread.pc == MATCH_PC {
                    if d <= stop {
                        // The highest-priority match so far; everything
                        // after it in the list ranks lower.
                        matched = Some((thread.start, d, thread.caps.clone()));
                        break;
                    }
                    continue;
                }
                advance(&vm, scratch, &mut nlist, thread, d);
            }
            std::mem::swap(&mut clist, &mut nlist);
            match cur_len {
                Some(len) => d += len,
                None => {
                    // The end of the text (or of the region): live threads
                    // can consume nothing more, so only matches recorded so
                    // far count, and no later candidate exists.
                    break match matched.take() {
                        Some((start, end, caps)) => {
                            SearchOutcome::Found(start, registers(vm.num_regs, start, end, &caps))
                        }
                        None => SearchOutcome::NotFound,
                    };
                }
            }
        };
        scratch.clist = clist;
        scratch.nlist = nlist;
        outcome
    })
}
