//! Live call metadata. No lexical environment or Lisp source representation is retained.
use super::*;

pub(super) struct Activation {
    pub token: usize,
    pub function: Value,
    resolved: Value,
    args: std::ops::Range<usize>,
    pub dynamic: usize,
}
#[derive(Default)]
pub(super) struct Backtrace {
    pub frames: Vec<Activation>,
    arguments: Vec<Value>,
    next_token: usize,
    pub bindings: Vec<(usize, Value)>,
    /// While a world build observes startup: the token counter, parked here
    /// so that `next_token` stays at its maximum and every call takes the
    /// counter's overflow path, which records the symbol called through.
    /// Symbols are stable slot IDs, so the record needs no tracing.
    observed: Option<(usize, Vec<usize>)>,
}
impl Backtrace {
    pub fn enter(
        &mut self,
        function: Value,
        resolved: Value,
        args: &[Value],
        dynamic: usize,
    ) -> usize {
        let mark = self.frames.len();
        let start = self.arguments.len();
        self.arguments.extend_from_slice(args);
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
            args: start..self.arguments.len(),
            dynamic,
        });
        mark
    }
    /// A call while startup is observed: its token, from the parked counter.
    #[cold]
    #[inline(never)]
    fn observed_call(&mut self, function: Value) -> usize {
        let (counter, calls) = self.observed.as_mut().expect("backtrace token exhausted");
        *counter = counter.checked_add(1).expect("backtrace token exhausted");
        calls.extend(function.index_if(7).and_then(|i| i.checked_sub(1)));
        *counter
    }
    /// Record the functions every call names, from now until `observed`.
    pub fn observe(&mut self) {
        self.observed = Some((std::mem::replace(&mut self.next_token, usize::MAX), Vec::new()));
    }
    /// The symbols called through since `observe`, in call order, and the
    /// counter back.
    pub fn observed(&mut self) -> Vec<usize> {
        let (counter, calls) = self.observed.take().expect("observing calls");
        self.next_token = counter;
        calls
    }
    pub fn resolve_pending(&mut self, resolved: Value) {
        self.frames
            .last_mut()
            .expect("pending call activation")
            .resolved = resolved;
    }
    pub fn leave(&mut self, mark: usize) {
        if let Some(frame) = self.frames.get(mark) {
            self.arguments.truncate(frame.args.start);
        }
        self.frames.truncate(mark);
    }
    pub fn arguments(&self, index: usize) -> &[Value] {
        &self.arguments[self.frames[index].args.clone()]
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
    }
}
impl Vm {
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
        let symbol = self.backtrace.bindings.iter().find_map(|(depth, symbol)| (*depth == at).then_some(*symbol))?;
        let base = symbol.index_if(7)? - 1;
        (self.local_binding_slots.contains(&slot) && self.localized_boolean(base).is_some()).then_some((base, owner))
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
    fn backtrace_binding_value(&mut self, at: usize) -> Result<Value> {
        let (slot, _, owner, default) = self.dynamic[at];
        if let Some((base, owner)) = self.backtrace_boolean_local(at) {
            for next in at + 1..self.dynamic.len() {
                if self.backtrace_boolean_local(next) == Some((base, owner)) { return Ok(self.dynamic[next].1); }
            }
            let local = self.declaration_hash_get(self.slots[base].local, owner)?.ok_or("missing local binding")?;
            return Ok(self.slots[local.index(7)? - 1].value);
        }
        let fwd = self.localized_boolean(slot).or_else(|| self.forwarding_ref(slot));
        for &(next, old, next_owner, next_default) in &self.dynamic[at + 1..] {
            if next != slot {
                continue;
            }
            if matches!(fwd, Some(forwarded::Fwd::Buffer(_)))
                && (default != next_default || (!default && owner != next_owner))
            {
                continue;
            }
            return Ok(old);
        }
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
        let start = self.backtrace.frames[index].dynamic;
        let end = self.backtrace.frames[index + 1].dynamic;
        let bindings: Vec<_> = self
            .backtrace
            .bindings
            .iter()
            .copied()
            .filter(|(at, _)| *at >= start && *at < end)
            .collect();
        let mut values = Vec::with_capacity(bindings.len());
        for (at, symbol) in bindings {
            // GNU unrewind omits LET_LOCAL entries whose storage disappeared.
            if !self
                .backtrace_binding_live(at, symbol)
                .map_err(prim::Fault::Error)?
            {
                continue;
            }
            let value = self
                .backtrace_binding_value(at)
                .map_err(prim::Fault::Error)?;
            self.conses.push((symbol, value));
            values.push(Value::reference(self.conses.len() - 1, 3));
        }
        Ok(self.list_value(&values))
    }
}
