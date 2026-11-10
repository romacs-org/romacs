//! Coarse list kernels shared by sealed handlers and indirect primitive calls.
use super::*;

impl Vm {
    pub(crate) fn traversal_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        let arity = if matches!(p, Primitive::Car | Primitive::Cdr | Primitive::Null) {
            1
        } else {
            2
        };
        if n != arity {
            return Err("wrong primitive arity".into());
        }
        let x = self.stack[start];
        let y = if n > 1 {
            self.stack[start + 1]
        } else {
            Value::NIL
        };
        self.traversal(p, x, y)
    }
    #[inline(always)]
    pub(crate) fn traversal(&mut self, p: Primitive, x: Value, y: Value) -> Result<Value> {
        use Primitive::*;
        match p {
            Null => Ok(if x == Value::NIL {
                Value::TRUE
            } else {
                Value::NIL
            }),
            Eq => Ok(if x == y { Value::TRUE } else { Value::NIL }),
            Car | Cdr => {
                if x == Value::NIL {
                    return Ok(x);
                }
                let pair = self.cons_pair(x)?;
                Ok(if p == Car { pair.0 } else { pair.1 })
            }
            Setcar | Setcdr => {
                let i = self.writable_cons(x)?;
                if p == Setcar {
                    self.conses.set_car(i, y);
                } else {
                    self.conses.set_cdr(i, y);
                }
                Ok(y)
            }
            Nth => {
                let count = match self.bigint_value(x).filter(|_| x.integer_value().is_none()) {
                    Some(big) => prim::prims::fns::big_nth_count(&big, y, |t| self.cons_pair(t).ok().map(|p| p.1)) as usize,
                    None => x.as_integer()?.max(0) as usize,
                };
                let mut tail = y;
                // Brent's cycle detector is needed only for counts larger than
                // the heap's possible proper-list length. Reduce the remaining
                // count modulo the cycle, rather than rejecting a valid nth.
                let mut anchor = tail;
                let mut power = 1usize;
                let mut period = 0usize;
                let mut remaining = count;
                while remaining != 0 {
                    if tail == Value::NIL {
                        return Ok(tail);
                    }
                    tail = self.cons_pair(tail)?.1;
                    remaining -= 1;
                    if count > self.list_limit() {
                        period += 1;
                        if tail == anchor {
                            remaining %= period;
                        } else if period == power {
                            anchor = tail;
                            power = power.saturating_mul(2);
                            period = 0;
                        }
                    }
                }
                Ok(if tail == Value::NIL {
                    tail
                } else {
                    self.cons_pair(tail)?.0
                })
            }
            Memq | Assq => {
                let (key, mut tail) = (x, y);
                // Single mutator: neither the arena length nor the mapped-image
                // size can change during this kernel. Keep the circularity bound.
                let limit = self.list_limit();
                let mut steps = 0;
                while tail != Value::NIL {
                    let (item, next) = self.cons_pair(tail)?;
                    if p == Memq {
                        if item == key {
                            return Ok(tail);
                        }
                    } else if item.tag() == 3 && self.cons_pair(item)?.0 == key {
                        return Ok(item);
                    }
                    tail = next;
                    steps += 1;
                    if steps > limit {
                        return Err("circular list".into());
                    }
                }
                Ok(Value::NIL)
            }
            _ => unreachable!("traversal dispatch"),
        }
    }
}

impl Vm {
    /// The length of the proper list LIST, else UNBOUND: a dotted or
    /// circular list is the primitive's to report.
    #[inline(never)]
    pub(super) fn list_length_kernel(&self, list: Value) -> Value {
        let (mut tail, mut length, limit) = (list, 0usize, self.list_limit());
        while let Some((_, next)) = pair(self, tail) {
            tail = next;
            length += 1;
            if length > limit {
                return Value::UNBOUND;
            }
        }
        if tail == Value::NIL { Value::integer(length as i64).unwrap_or(Value::UNBOUND) } else { Value::UNBOUND }
    }
    /// `nthcdr' of a count below GNU's SMALL_LIST_LEN_MAX on a plain walk,
    /// else UNBOUND.
    #[inline(never)]
    pub(super) fn nthcdr_kernel(&self, count: Value, list: Value) -> Value {
        let Some(count) = count.integer_value().filter(|&n| n <= 127) else { return Value::UNBOUND };
        let mut tail = list;
        for _ in 0..count.max(0) {
            match pair(self, tail) {
                Some((_, next)) => tail = next,
                None if tail == Value::NIL => return Value::NIL,
                None => return Value::UNBOUND,
            }
        }
        tail
    }
    /// `member' over a proper list, else UNBOUND.
    #[inline(never)]
    pub(super) fn member_kernel(&self, element: Value, list: Value) -> Value {
        let (mut tail, mut steps, limit) = (list, 0usize, self.list_limit());
        while let Some((item, next)) = pair(self, tail) {
            match self.equal_kernel(element, item) {
                Value::TRUE => return tail,
                Value::UNBOUND => return Value::UNBOUND,
                _ => {}
            }
            tail = next;
            steps += 1;
            if steps > limit {
                return Value::UNBOUND;
            }
        }
        if tail == Value::NIL { Value::NIL } else { Value::UNBOUND }
    }
    /// `get' of an ordinary property, else UNBOUND: a fence observes the
    /// read, and the condition properties live in the registry.
    #[inline(never)]
    pub(super) fn get_kernel(&self, symbol: Value, property: Value) -> Value {
        let registry = property.index_if(7).and_then(|i| i.checked_sub(1)).and_then(|slot| self.symbol_names.get(slot))
            .is_some_and(|name| name == "error-conditions" || name == "error-message");
        if registry || self.fence.is_some() {
            return Value::UNBOUND;
        }
        self.symbol_property(symbol, property).unwrap_or(Value::UNBOUND)
    }
}

// Keep error formatting and image-authentication stack addresses out of the
// tail-threaded handler. Even a cold address escape can inhibit LLVM tail calls.
#[inline(never)]
fn mapped_pair(vm: &Vm, value: Value) -> Option<(Value, Value)> {
    vm.cons_pair(value).ok()
}
#[inline(always)]
pub(super) fn pair(vm: &Vm, value: Value) -> Option<(Value, Value)> {
    if value.tag() != 3 {
        return None;
    }
    if Vm::image_value(value) {
        return mapped_pair(vm, value);
    }
    vm.conses.get(value.payload() as usize).copied()
}
#[inline(always)]
fn kernel<const P: usize>(vm: &mut Vm, x: Value, y: Value) -> Option<Value> {
    use Primitive::*;
    let p = prim::descriptor(P)?.legacy()?;
    match p {
        Null => Some(if x == Value::NIL {
            Value::TRUE
        } else {
            Value::NIL
        }),
        Eq => Some(if x == y { Value::TRUE } else { Value::NIL }),
        Car | Cdr => {
            if x == Value::NIL {
                return Some(x);
            }
            let (car, cdr) = pair(vm, x)?;
            Some(if p == Car { car } else { cdr })
        }
        Setcar | Setcdr => {
            if x.tag() != 3 || Vm::image_value(x) {
                return None;
            }
            let i = x.payload() as usize;
            if i >= vm.conses.len() {
                return None;
            }
            if p == Setcar {
                vm.conses.set_car(i, y);
            } else {
                vm.conses.set_cdr(i, y);
            }
            Some(y)
        }
        Nth => {
            let count = x.integer_value()?.max(0) as usize;
            if count > vm.list_limit() {
                return None;
            }
            let mut tail = y;
            for _ in 0..count {
                // Nil or malformed tails take the shared diagnostic path.
                tail = pair(vm, tail)?.1;
            }
            Some(if tail == Value::NIL {
                tail
            } else {
                pair(vm, tail)?.0
            })
        }
        Memq | Assq => {
            let (key, mut tail) = (x, y);
            let limit = vm.list_limit();
            let mut steps = 0;
            while tail != Value::NIL {
                let (item, next) = pair(vm, tail)?;
                if p == Memq {
                    if item == key {
                        return Some(tail);
                    }
                } else if item.tag() == 3 && pair(vm, item)?.0 == key {
                    return Some(item);
                }
                tail = next;
                steps += 1;
                if steps > limit {
                    return None;
                }
            }
            Some(Value::NIL)
        }
        _ => unreachable!("traversal handler"),
    }
}

/// `car' or `cdr' of a private cons or nil, with no stack frame: tag bits
/// alone say cons, the image bit puts an index past the arena, and whatever
/// else arrives leaves by tail call.
macro_rules! half {
    ($name:ident, $general:ident, $field:tt) => {
        /// # Safety
        /// R is the running engine's current frame (`Engine::regs`) and PC an
        /// instruction of the function running in it: the registers and constants
        /// its operands name lie in that frame and code, as lowering checked.
        pub(super) unsafe fn $name(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            // SAFETY: R and PC as the function's contract (`# Safety`).
            unsafe {
                let i = &*pc;
                let x = *r.add(i.d);
                if x.0 & 7 == 3 {
                    let index = (x.0 >> 3) as usize;
                    if let Some(pair) = e.vm.conses.get(index) {
                        *r.add(i.a) = pair.$field;
                        return advance(e, r, pc);
                    }
                } else if x == Value::NIL {
                    *r.add(i.a) = Value::NIL;
                    return advance(e, r, pc);
                }
                $general(e, r, pc)
            }
        }
    };
}
macro_rules! handler {
    ($(#[$attribute:meta])* $name:ident, $p:ident) => {
        $(#[$attribute])*
        /// # Safety
        /// R is the running engine's current frame (`Engine::regs`) and PC an
        /// instruction of the function running in it: the registers and constants
        /// its operands name lie in that frame and code, as lowering checked.
        pub(super) unsafe fn $name(mut e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            // SAFETY: R and PC as the function's contract (`# Safety`).
            unsafe {
                let i = &*pc;
                let x = *r.add(i.d);
                let y = if i.e > 1 { *r.add(i.d + 1) } else { Value::NIL };
                let Some(value) = kernel::<{ Primitive::$p as usize }>(e.vm, x, y) else {
                    return sealed_call(e, r, pc);
                };
                *r.add(i.a) = value;
                advance(e, r, pc)
            }
        }
    };
}
// Never inlined into `car' and `cdr': their calls would give those a frame.
handler!(#[inline(never)] mapped_car, Car);
handler!(#[inline(never)] mapped_cdr, Cdr);
half!(car, mapped_car, 0);
half!(cdr, mapped_cdr, 1);
handler!(null, Null);
handler!(eq, Eq);
handler!(memq, Memq);
handler!(assq, Assq);
// GNU plist_get, the eq walk of a two-argument `plist-get'.
/// # Safety
/// R is the running engine's current frame (`Engine::regs`) and PC an
/// instruction of the function running in it: the registers and constants
/// its operands name lie in that frame and code, as lowering checked.
pub(super) unsafe fn plist_get(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
    // SAFETY: R and PC as the function's contract (`# Safety`).
    unsafe {
        let i = &*pc;
        let Some(value) = plist_get_eq(e.vm, *r.add(i.d), *r.add(i.d + 1)) else {
            return sealed_call(e, r, pc);
        };
        *r.add(i.a) = value;
        advance(e, r, pc)
    }
}
#[inline(always)]
fn plist_get_eq(vm: &Vm, mut tail: Value, key: Value) -> Option<Value> {
    let limit = vm.list_limit();
    let mut steps = 0;
    while tail.tag() == 3 {
        let (item, next) = pair(vm, tail)?;
        if next.tag() != 3 {
            return Some(Value::NIL);
        }
        let (value, rest) = pair(vm, next)?;
        if item == key {
            return Some(value);
        }
        tail = rest;
        steps += 1;
        if steps > limit {
            return Some(Value::NIL);
        }
    }
    Some(Value::NIL)
}
handler!(nth, Nth);
handler!(setcar, Setcar);
handler!(setcdr, Setcdr);
