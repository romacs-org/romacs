//! Live call metadata. No lexical environment or Lisp source representation is retained.
//! Every reader of the call stack goes through these activations: GNU's
//! backtrace primitives, `backtrace-eval`'s unrewind and the debugger.
use super::*;

/// Where a call's arguments are read from, as GNU's `record_in_backtrace`
/// keeps a pointer to the caller's pushed values (bytecode.c `Bcall`).
pub(super) enum Arguments {
    /// The caller's argument registers: OFFSET..OFFSET+COUNT of the register
    /// stack at suspension depth STACK (`Vm::suspended_stacks` index; the live
    /// stack is at `suspended_stacks.len()`). Nothing writes them while the
    /// callee runs; widening keeps offsets; collection forwards them as
    /// registers; a nested run sets the whole stack aside unmoved.
    Registers { stack: usize, offset: usize, count: usize },
    /// Copied into `Backtrace::arguments`: a call through `apply`, whose
    /// spread window a nested `apply` reuses while the callee runs.
    Copied(std::ops::Range<usize>),
}
romacs_snapshot::snapshot_enum!(Arguments {
    Arguments::Registers { stack, offset, count } => (stack, offset, count),
    Arguments::Copied(f0) => (f0),
});
pub(super) struct Activation {
    pub token: usize,
    pub function: Value,
    resolved: Value,
    args: Arguments,
    pub dynamic: usize,
}
romacs_snapshot::snapshot_struct!(Activation { token, function, resolved, args, dynamic });
/// A `save-excursion` in progress, for `backtrace-eval`'s unrewind (GNU
/// SPECPDL_UNWIND_EXCURSION): its marker, the selected window when it
/// showed the marker's buffer (else nil), and where it was made, as the
/// activation count and the binding depth then. The pair orders it against
/// both: one made in a caller just before a call is outside the callee's
/// entry even at the same binding depth.
pub(super) struct Excursion {
    pub marker: Value,
    pub window: Value,
    pub frames: usize,
    pub dynamic: usize,
}
romacs_snapshot::snapshot_struct!(Excursion { marker, window, frames, dynamic });
#[derive(Default)]
pub(super) struct Backtrace {
    pub frames: Vec<Activation>,
    arguments: Vec<Value>,
    next_token: usize,
    /// The named dynamic bindings, as (depth, symbol), in depth order.
    pub bindings: Vec<(usize, Value)>,
    /// The tokens of activations whose debug-on-exit flag is set
    /// (`backtrace-debug`). Tokens are never reused, so a stale one names
    /// no activation.
    exit_flags: std::collections::BTreeSet<usize>,
    /// `exit_flags`' length when its stale tokens were last dropped.
    exit_pruned: usize,
    /// The `save-excursion`s in progress, innermost last.
    pub excursions: Vec<Excursion>,
    /// While a world build observes startup: the token counter, parked here
    /// so that `next_token` stays at its maximum and every call takes the
    /// counter's overflow path, which records the symbol called through.
    /// Symbols are stable slot IDs, so the record needs no tracing.
    observed: Option<(usize, Vec<usize>)>,
    /// The only symbols an observation records, sorted (`observe_only`);
    /// empty records every one.
    only: Vec<usize>,
}
romacs_snapshot::snapshot_struct!(Backtrace { frames, arguments, next_token, bindings, exit_flags, exit_pruned, excursions, observed, only });
impl Backtrace {
    pub fn enter(
        &mut self,
        function: Value,
        resolved: Value,
        args: Arguments,
        dynamic: usize,
    ) -> usize {
        let mark = self.frames.len();
        let token = match self.next_token.checked_add(1) {
            Some(token) => {
                self.next_token = token;
                token
            }
            None => self.observed_call(function),
        };
        self.frames.push(Activation {
            token,
            function,
            resolved,
            args,
            dynamic,
        });
        mark
    }
    /// ARGS copied, for a call whose registers do not outlive its entry.
    pub fn copied(&mut self, args: &[Value]) -> Arguments {
        let start = self.arguments.len();
        self.arguments.extend_from_slice(args);
        Arguments::Copied(start..self.arguments.len())
    }
    /// A call while startup is observed: its token, from the parked counter.
    #[cold]
    #[inline(never)]
    fn observed_call(&mut self, function: Value) -> usize {
        let (counter, calls) = self.observed.as_mut().expect("backtrace token exhausted");
        *counter = counter.checked_add(1).expect("backtrace token exhausted");
        let only = &self.only;
        calls.extend(function.index_if(7).and_then(|i| i.checked_sub(1)).filter(|slot| only.is_empty() || only.binary_search(slot).is_ok()));
        *counter
    }
    /// Whether calls are being observed (`observe`).
    pub fn observing(&self) -> bool {
        self.observed.is_some()
    }
    /// Record the calls through ONLY's symbols, from now until `observed`.
    pub fn observe_only(&mut self, mut only: Vec<usize>) {
        only.sort_unstable();
        self.observe();
        self.only = only;
    }
    /// Whether the observation so far recorded a call.
    pub fn observed_any(&self) -> bool {
        self.observed.as_ref().is_some_and(|(_, calls)| !calls.is_empty())
    }
    /// Record the functions every call names, from now until `observed`.
    pub fn observe(&mut self) {
        self.only.clear();
        self.observed = Some((std::mem::replace(&mut self.next_token, usize::MAX), Vec::new()));
    }
    /// The symbols called through since `observe`, in call order, and the
    /// counter back.
    pub fn observed(&mut self) -> Vec<usize> {
        let (counter, calls) = self.observed.take().expect("observing calls");
        self.next_token = counter;
        self.only.clear();
        calls
    }
    pub fn resolve_pending(&mut self, resolved: Value) {
        self.frames
            .last_mut()
            .expect("pending call activation")
            .resolved = resolved;
    }
    pub fn leave(&mut self, mark: usize) {
        let copied = self.frames.get(mark..).into_iter().flatten().find_map(|frame| match &frame.args {
            Arguments::Copied(range) => Some(range.start),
            Arguments::Registers { .. } => None,
        });
        if let Some(start) = copied {
            self.arguments.truncate(start);
        }
        self.frames.truncate(mark);
    }
    pub fn bind(&mut self, at: usize, symbol: Value) {
        self.bindings.push((at, symbol));
    }
    pub fn unbind(&mut self, depth: usize) {
        while self.bindings.last().is_some_and(|(at, _)| *at >= depth) {
            self.bindings.pop();
        }
    }
    pub fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for frame in &mut self.frames {
            visit(&mut frame.function);
            visit(&mut frame.resolved);
        }
        for value in &mut self.arguments {
            visit(value);
        }
        for (_, symbol) in &mut self.bindings {
            visit(symbol);
        }
        for excursion in &mut self.excursions {
            visit(&mut excursion.marker);
            visit(&mut excursion.window);
        }
    }
    /// The stack index of the activation TOKEN names: tokens increase with
    /// the index, so a binary search finds it.
    pub fn index_of(&self, token: usize) -> Option<usize> {
        self.frames.binary_search_by_key(&token, |frame| frame.token).ok()
    }
    /// The symbol of the named binding at dynamic depth AT, if it has one.
    pub fn binding_symbol(&self, at: usize) -> Option<Value> {
        self.bindings.binary_search_by_key(&at, |(depth, _)| *depth).ok().map(|i| self.bindings[i].1)
    }
    /// The named bindings made at depths START..END.
    pub fn binding_range(&self, start: usize, end: usize) -> &[(usize, Value)] {
        let lo = self.bindings.partition_point(|(at, _)| *at < start);
        let hi = self.bindings.partition_point(|(at, _)| *at < end);
        &self.bindings[lo..hi]
    }
    pub fn debug_on_exit(&self, token: usize) -> bool {
        self.exit_flags.contains(&token)
    }
    /// Whether any activation's debug-on-exit flag is set.
    pub fn has_exit_flags(&self) -> bool {
        !self.exit_flags.is_empty()
    }
    /// Whether an activation from MARK up is flagged debug-on-exit. Tokens
    /// grow with the stack, so those are the tokens from MARK's.
    pub fn exit_flagged_from(&self, mark: usize) -> bool {
        self.frames.get(mark).is_some_and(|frame| self.exit_flags.range(frame.token..).next().is_some())
    }
    /// Drop the flags of the activations from MARK up, which are leaving.
    pub fn drop_exit_flags_from(&mut self, mark: usize) {
        if let Some(frame) = self.frames.get(mark) {
            self.exit_flags.split_off(&frame.token);
        }
    }
    /// GNU set_backtrace_debug_on_exit for the activation TOKEN names. The
    /// tokens of activations that left stay until the set has doubled since
    /// they were last dropped, so a flag costs O(log n) amortized.
    pub fn set_debug_on_exit(&mut self, token: usize, on: bool) {
        if on { self.exit_flags.insert(token); } else { self.exit_flags.remove(&token); }
        if self.exit_flags.len() > 2 * self.exit_pruned.max(32) {
            let frames = &self.frames;
            self.exit_flags.retain(|t| frames.binary_search_by_key(t, |frame| frame.token).is_ok());
            self.exit_pruned = self.exit_flags.len();
        }
    }
    /// A `save-excursion` begins in the activation FRAMES - 1, at binding
    /// depth DYNAMIC: record MARKER and WINDOW there.
    pub fn excursion_begin(&mut self, marker: Value, window: Value, frames: usize, dynamic: usize) {
        self.excursions.push(Excursion { marker, window, frames, dynamic });
    }
    /// The `save-excursion` of MARKER ends: drop its record, and any newer
    /// one an abandoned body left.
    pub fn excursion_end(&mut self, marker: Value) {
        if let Some(at) = self.excursions.iter().rposition(|e| e.marker == marker) {
            self.excursions.truncate(at);
        }
    }
}
/// The variable a disabled binding binds (`Vm::backtrace_disable_binding`).
const INERT_BINDING: &str = "#:vm2-inert-binding";
/// One step of `backtrace-eval`'s unrewind, in the order the stack made
/// them: a dynamic binding at a depth, or an excursion record.
#[derive(Clone, Copy)]
enum Unrewind {
    Binding(usize),
    Excursion(usize),
}
impl Vm {
    /// The arguments of activation INDEX, where they were pushed.
    pub(super) fn backtrace_arguments(&self, index: usize) -> &[Value] {
        match &self.backtrace.frames[index].args {
            Arguments::Copied(range) => &self.backtrace.arguments[range.clone()],
            &Arguments::Registers { stack, offset, count } => {
                let stack = self.suspended_stacks.get(stack).unwrap_or(&self.stack);
                &stack[offset..offset + count]
            }
        }
    }
    /// `backtrace_arguments' for a reader that must not panic.
    pub(super) fn backtrace_arguments_if(&self, index: usize) -> Option<&[Value]> {
        match &self.backtrace.frames.get(index)?.args {
            Arguments::Copied(range) => self.backtrace.arguments.get(range.clone()),
            &Arguments::Registers { stack, offset, count } => {
                self.suspended_stacks.get(stack).unwrap_or(&self.stack).get(offset..offset.checked_add(count)?)
            }
        }
    }
    pub(super) fn backtrace_resolve(
        &self,
        original: Value,
    ) -> std::result::Result<Value, prim::Fault> {
        let mut function = original;
        for _ in 0..=self.slots.len() {
            if function.tag() != 7 || function == Value::UNBOUND {
                return Ok(function);
            }
            let Some(slot) = function
                .payload()
                .checked_sub(1)
                .and_then(|i| self.slots.get(i as usize))
            else {
                return Ok(function);
            };
            function = slot.function;
        }
        Err(prim::Fault::CyclicFunctionIndirection(original))
    }
    pub(super) fn backtrace_start(
        &self,
        mut base: Value,
    ) -> std::result::Result<Option<usize>, prim::Fault> {
        let unbased = base == Value::NIL;
        let mut offset = 0;
        if let Ok((n, rest)) = self.cons_pair(base)
            && let Some(n) = n.integer_value()
        {
            offset = n.max(0) as usize;
            base = rest;
        }
        let index = if unbased {
            self.backtrace.frames.len().checked_sub(1)
        } else {
            let target = self.backtrace_resolve(base)?;
            let mut found = None;
            for (i, frame) in self.backtrace.frames.iter().enumerate().rev() {
                if self.backtrace_resolve(frame.function)? == target {
                    found = Some(i);
                    break;
                }
            }
            found
        };
        Ok(index.and_then(|i| i.checked_sub(offset)))
    }
    fn backtrace_boolean_local(&self, at: usize) -> Option<(usize, Value)> {
        let (slot, _, owner, _) = self.dynamic[at];
        let symbol = self.backtrace.binding_symbol(at)?;
        let base = symbol.index_if(7)? - 1;
        (self.local_binding_slots.contains(&slot) && self.localized_forwarded(base).is_some()).then_some((base, owner))
    }
    fn backtrace_binding_live(&mut self, at: usize, symbol: Value) -> Result<bool> {
        let (slot, _, owner, default) = self.dynamic[at];
        if let Some((base, owner)) = self.backtrace_boolean_local(at) {
            return Ok(self.live_buffer_value(owner) && self.declaration_hash_get(self.slots[base].local, owner)?.is_some());
        }
        match self.forwarding_ref(slot) {
            Some(forwarded::Fwd::Buffer(i)) if !default => Ok(self.live_buffer_value(owner)
                && owner
                    .index_if(0)
                    .is_some_and(|index| self.per_buffer_local_p(index - 1, i))),
            None if symbol.index(7)? - 1 != slot => {
                if !self.live_buffer_value(owner) {
                    return Ok(false);
                }
                let Some(table) = self.local_table(symbol.index(7)? - 1)? else {
                    return Ok(false);
                };
                Ok(self.declaration_hash_get(table, owner)? == Some(Value::reference(slot + 1, 7)))
            }
            _ => Ok(true),
        }
    }
    /// What makes the binding at AT and a newer one bindings of the same
    /// variable: its slot, and for a per-buffer variable also whether it is
    /// the default and whose local it is.
    fn backtrace_shadow_key(&self, at: usize) -> (usize, bool, u64) {
        let (slot, _, owner, default) = self.dynamic[at];
        let fwd = self.localized_forwarded(slot).or_else(|| self.forwarding_ref(slot));
        match fwd {
            Some(forwarded::Fwd::Buffer(_)) => (slot, default, if default { Value::NIL.0 } else { owner.0 }),
            _ => (slot, false, Value::NIL.0),
        }
    }
    /// The binding that saved the value of the one at AT: the nearest newer
    /// binding of the same variable. A scan of the newer bindings, as GNU's
    /// `backtrace--locals` unrewinds the whole distance per call;
    /// `backtrace_successors` gives every binding's in one sweep.
    fn backtrace_successor(&self, at: usize) -> Option<usize> {
        if let Some(local) = self.backtrace_boolean_local(at) {
            return (at + 1..self.dynamic.len()).find(|&next| self.backtrace_boolean_local(next) == Some(local));
        }
        let key = self.backtrace_shadow_key(at);
        (at + 1..self.dynamic.len()).find(|&next| self.backtrace_shadow_key(next) == key)
    }
    /// Every binding's `backtrace_successor`, from the newest down: O(bindings).
    fn backtrace_successors(&self) -> Vec<Option<usize>> {
        let mut successors = vec![None; self.dynamic.len()];
        let mut newer = std::collections::HashMap::new();
        let mut newer_local = std::collections::HashMap::new();
        for at in (0..self.dynamic.len()).rev() {
            let key = self.backtrace_shadow_key(at);
            successors[at] = match self.backtrace_boolean_local(at) {
                Some(local) => newer_local.insert((local.0, local.1.0), at),
                None => newer.get(&key).copied(),
            };
            newer.insert(key, at);
        }
        successors
    }
    /// The value of the binding at AT, which SUCCESSOR saved if it is set.
    fn backtrace_binding_value(&mut self, at: usize, successor: Option<usize>) -> Result<Value> {
        let (slot, _, owner, default) = self.dynamic[at];
        if let Some((base, owner)) = self.backtrace_boolean_local(at) {
            if let Some(next) = successor { return Ok(self.dynamic[next].1); }
            let local = self.declaration_hash_get(self.slots[base].local, owner)?.ok_or("missing local binding")?;
            return Ok(self.slots[local.index(7)? - 1].value);
        }
        self.fence_binding(slot, at, owner);
        match successor {
            Some(next) => Ok(self.dynamic[next].1),
            None => self.unfenced(slot, |vm| vm.backtrace_live_value(slot, owner, default)),
        }
    }
    /// Activation INDEX's own bindings, live ones only, as (SYMBOL . VALUE)
    /// conses, oldest first (GNU unrewind omits LET_LOCAL entries whose
    /// storage disappeared). SUCCESSORS, when given, are the sweep's.
    fn backtrace_frame_locals(&mut self, index: usize, successors: Option<&[Option<usize>]>) -> Result<Vec<Value>> {
        let start = self.backtrace.frames[index].dynamic;
        let end = self.backtrace.frames[index + 1].dynamic;
        let bindings = self.backtrace.binding_range(start, end).to_vec();
        let mut values = Vec::with_capacity(bindings.len());
        let inert = self.intern(INERT_BINDING);
        for (at, symbol) in bindings {
            if symbol == inert || !self.backtrace_binding_live(at, symbol)? {
                continue;
            }
            let successor = match successors {
                Some(all) => all[at],
                None => self.backtrace_successor(at),
            };
            let value = self.backtrace_binding_value(at, successor)?;
            values.push(self.alloc_cons(symbol, value));
        }
        Ok(values)
    }
    /// The current value of the binding of SLOT that OWNER and DEFAULT name.
    fn backtrace_live_value(&mut self, slot: usize, owner: Value, default: bool) -> Result<Value> {
        let fwd = self.localized_forwarded(slot).or_else(|| self.forwarding_ref(slot));
        match fwd {
            Some(forwarded::Fwd::Buffer(i)) if !default => {
                let buffer = owner
                    .index_if(0)
                    .and_then(|n| n.checked_sub(1))
                    .ok_or("buffer required")?;
                Ok(self.forwarded_buffer_value(buffer, i))
            }
            Some(fwd) if default => self.forwarded_default(fwd),
            Some(fwd) => self.forwarded_value(fwd),
            None => Ok(self.slots[slot].value),
        }
    }
    pub(super) fn backtrace_locals(&mut self, n: Value, base: Value) -> prim::Out {
        let level = n
            .integer_value()
            .filter(|n| *n >= 0)
            .ok_or(prim::Fault::Wrong("wholenump", n))? as usize;
        if level == 0 {
            return Err(prim::Fault::Wrong("wholenump", Value::integer(-1).unwrap()));
        }
        let index = self
            .backtrace_start(base)?
            .and_then(|i| i.checked_sub(level))
            .ok_or_else(|| prim::Fault::Error("Activation frame not found!".into()))?;
        let values = self.backtrace_frame_locals(index, None).map_err(prim::Fault::Error)?;
        Ok(self.list_value(&values))
    }
    /// GNU get_backtrace_frame: the stack index of the activation NFRAMES
    /// out from BASE's (the innermost when BASE is nil), if there is one.
    pub(super) fn backtrace_frame_index(&self, nframes: Value, base: Value) -> std::result::Result<Option<usize>, prim::Fault> {
        let count = nframes.integer_value().filter(|&n| n >= 0).ok_or(prim::Fault::Wrong("wholenump", nframes))?;
        Ok(self.backtrace_start(base)?.and_then(|i| i.checked_sub(usize::try_from(count).ok()?)))
    }
    /// An activation's FLAGS as `mapbacktrace` passes them.
    pub(super) fn backtrace_flags(&mut self, index: usize) -> Value {
        if self.backtrace.debug_on_exit(self.backtrace.frames[index].token) {
            let key = self.intern(":debug-on-exit");
            self.list_value(&[key, Value::TRUE])
        } else {
            Value::NIL
        }
    }
    /// `mapbacktrace`'s (EVALD FUNCTION ARGS FLAGS) for activation INDEX.
    pub(super) fn backtrace_frame_values(&mut self, index: usize) -> [Value; 4] {
        let function = self.backtrace.frames[index].function;
        let args = self.backtrace_arguments(index).to_vec();
        let args = self.list_value(&args);
        let flags = self.backtrace_flags(index);
        [Value::TRUE, function, args, flags]
    }
    /// `backtrace--frames-and-locals`: every activation from BASE's outward
    /// as (EVALD FUNCTION ARGS FLAGS LOCALS), LOCALS as `backtrace--locals`
    /// gives them and nil for BASE's own (as `backtrace-get-frames` leaves
    /// frame 0's), in one walk: O(frames + bindings).
    pub(super) fn backtrace_frames_and_locals(&mut self, base: Value) -> prim::Out {
        let Some(start) = self.backtrace_start(base)? else { return Ok(Value::NIL); };
        let successors = self.backtrace_successors();
        let mut frames = Vec::with_capacity(start + 1);
        for index in (0..=start).rev() {
            let locals = if index == start {
                Value::NIL
            } else {
                let values = self.backtrace_frame_locals(index, Some(&successors)).map_err(prim::Fault::Error)?;
                self.list_value(&values)
            };
            let [evald, function, args, flags] = self.backtrace_frame_values(index);
            frames.push(self.list_value(&[evald, function, args, flags, locals]));
        }
        Ok(self.list_value(&frames))
    }
    /// `backtrace-debug`: set activation INDEX's debug-on-exit flag.
    pub(super) fn backtrace_set_debug(&mut self, index: usize, on: bool) {
        let token = self.backtrace.frames[index].token;
        self.backtrace.set_debug_on_exit(token, on);
        self.debug_hooks_changed();
    }
    /// GNU specpdl_unrewind from the top of the stack to activation INDEX's
    /// entry, for `backtrace-eval`: each binding made since then, and each
    /// excursion begun inside INDEX or a newer activation, swaps its saved
    /// state with the current one, newest first. Watchers are not told
    /// (SET_INTERNAL_THREAD_SWITCH). What it swapped, for
    /// `backtrace_rewind`, which swaps it back in the other order.
    pub(super) fn backtrace_unrewind(&mut self, index: usize) -> Result<[usize; 4]> {
        if self.fence.is_some() {
            return Err("backtrace-eval while a world is built".into());
        }
        let span = [
            self.backtrace.frames[index].dynamic,
            self.dynamic.len(),
            self.backtrace.excursions.partition_point(|e| e.frames <= index),
            self.backtrace.excursions.len(),
        ];
        self.backtrace_swap(span, true)?;
        Ok(span)
    }
    pub(super) fn backtrace_rewind(&mut self, span: [usize; 4]) -> Result<()> {
        let [_, end, _, excursions] = span;
        if end > self.dynamic.len() || excursions > self.backtrace.excursions.len() {
            return Err("backtrace-eval's stack changed under it".into());
        }
        self.backtrace_swap(span, false)
    }
    /// SPAN's steps (bindings START..END, excursions FROM..TO) newest first
    /// when DOWN, else oldest first; a step that fails undoes the ones before.
    fn backtrace_swap(&mut self, [start, end, from, to]: [usize; 4], down: bool) -> Result<()> {
        let mut steps = Vec::with_capacity(end - start + to - from);
        let (mut at, mut k) = (end, to);
        while at > start || k > from {
            // An excursion begun once the stack held K's depth of bindings is
            // newer than the bindings below that depth.
            if k > from && (at == start || self.backtrace.excursions[k - 1].dynamic >= at) {
                k -= 1;
                steps.push(Unrewind::Excursion(k));
            } else {
                at -= 1;
                steps.push(Unrewind::Binding(at));
            }
        }
        if !down {
            steps.reverse();
        }
        let trapped = std::mem::take(&mut self.trapped);
        let mut result = Ok(());
        for (done, step) in steps.iter().enumerate() {
            if let Err(error) = self.backtrace_swap_step(*step) {
                for step in steps[..done].iter().rev() {
                    let _ = self.backtrace_swap_step(*step);
                }
                result = Err(error);
                break;
            }
        }
        self.trapped = trapped;
        result
    }
    fn backtrace_swap_step(&mut self, step: Unrewind) -> Result<()> {
        match step {
            Unrewind::Binding(at) => self.backtrace_swap_binding(at),
            Unrewind::Excursion(k) => self.backtrace_swap_excursion(k),
        }
    }
    /// One binding: the variable takes the value the binding saved, and the
    /// binding saves the one it had. A buffer's local binding is swapped
    /// while the buffer has one, made again since or not (GNU
    /// `local-variable-p`); one it has lost can be neither undone nor redone,
    /// so it becomes a binding of nothing (GNU SPECPDL_NOP).
    fn backtrace_swap_binding(&mut self, at: usize) -> Result<()> {
        let (slot, saved, owner, default) = self.dynamic[at];
        let symbol = self.backtrace.binding_symbol(at);
        let base = symbol.and_then(|symbol| symbol.index_if(7)).map(|index| index - 1);
        let present = match base {
            Some(base) if self.local_binding_slots.contains(&slot) => self.owner_local_binding(base, owner).is_some(),
            _ => match self.forwarding_ref(slot) {
                Some(forwarded::Fwd::Buffer(i)) if !default => self.live_buffer_value(owner)
                    && owner.index_if(0).is_some_and(|index| self.per_buffer_local_p(index - 1, i)),
                _ => true,
            },
        };
        if !present {
            self.backtrace_disable_binding(at);
            return Ok(());
        }
        let current = match base {
            Some(base) if self.local_binding_slots.contains(&slot) => {
                let target = self.owner_local_binding(base, owner).unwrap_or(base);
                self.slots[target].value
            }
            _ => self.backtrace_live_value(slot, owner, default)?,
        };
        let frame = self.root_frame(&[current]);
        let restored = self.restore_binding(slot, base, saved, owner, default);
        let [current] = self.release_frame(frame);
        restored?;
        self.dynamic[at].1 = current;
        Ok(())
    }
    /// The binding at AT binds nothing from now on: unbinding it, or
    /// swapping it again, changes no variable and tells no watcher. Its
    /// named-binding record stays in place, naming the inert variable,
    /// which no locals list shows.
    fn backtrace_disable_binding(&mut self, at: usize) {
        let nothing = self.intern(INERT_BINDING);
        let slot = nothing.index_if(7).expect("inert binding symbol") - 1;
        self.dynamic[at] = (slot, Value::NIL, Value::NIL, false);
        if let Ok(i) = self.backtrace.bindings.binary_search_by_key(&at, |(depth, _)| *depth) {
            self.backtrace.bindings[i].1 = nothing;
        }
    }
    /// The selected window when it shows the current buffer, else nil (GNU
    /// save_excursion_save).
    pub(super) fn excursion_window(&mut self) -> Result<Value> {
        let window = self.selected_window_value()?;
        Ok(if self.window(window)?.buffer == self.slots[self.buffer_slot].value { window } else { Value::NIL })
    }
    /// One excursion: GNU save_excursion_save of the current state into its
    /// record, then save_excursion_restore of the state it held: the saved
    /// buffer and point (none when that buffer was killed), and the saved
    /// window's point when another window is selected and it still shows
    /// that buffer.
    fn backtrace_swap_excursion(&mut self, k: usize) -> Result<()> {
        let (marker, window) = (self.backtrace.excursions[k].marker, self.backtrace.excursions[k].window);
        let current = self.slots[self.buffer_slot].value;
        let here = self.save_excursion_data()?;
        let here_window = self.excursion_window()?;
        let frame = self.root_frame(&[marker, window, here, current, here_window]);
        let mut result = self.backtrace_restore_excursion(marker, window);
        if result.is_ok() {
            let (marker, here, current) = (self.rooted(&frame, 0), self.rooted(&frame, 2), self.rooted(&frame, 3));
            result = self.set_marker_object(marker, here, current).map(drop);
        }
        let [_, _, here, _, here_window] = self.release_frame(frame);
        self.backtrace.excursions[k].window = here_window;
        let detached = self.set_marker_object(here, Value::NIL, Value::NIL).map(drop);
        result.and(detached)
    }
    fn backtrace_restore_excursion(&mut self, marker: Value, window: Value) -> Result<()> {
        let buffer = self.marker(marker)?.buffer;
        if buffer == Value::NIL || !self.live_buffer_value(buffer) {
            return Ok(());
        }
        self.restore_excursion_data(marker)?;
        let selected = self.selected_window_value()?;
        if window != Value::NIL && window != selected
            && matches!(self.object_if(window), Some(crate::data::Object::Window(w)) if w.live() && w.buffer == buffer)
        {
            // The point restored, clipped to the accessible portion.
            let here = self.save_excursion_data()?;
            let point = self.window(window)?.point;
            let set = self.set_marker_object(point, here, buffer).map(drop);
            self.set_marker_object(here, Value::NIL, Value::NIL)?;
            set?;
        }
        Ok(())
    }
}
