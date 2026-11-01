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
                let pair = self.cons_mut(x)?;
                if p == Setcar {
                    pair.0 = y;
                } else {
                    pair.1 = y;
                }
                Ok(y)
            }
            Nth => {
                let count = x.as_integer()?.max(0) as usize;
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
            let pair = vm.conses.get_mut(x.payload() as usize)?;
            if p == Setcar {
                pair.0 = y;
            } else {
                pair.1 = y;
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

macro_rules! handler {
    ($name:ident, $p:ident) => {
        pub(super) unsafe fn $name(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
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
handler!(car, Car);
handler!(cdr, Cdr);
handler!(null, Null);
handler!(eq, Eq);
handler!(memq, Memq);
handler!(assq, Assq);
// GNU plist_get, the eq walk of a two-argument `plist-get'.
pub(super) unsafe fn plist_get(e: &mut Engine<'_>, r: *mut Value, pc: *const Inst) -> Value {
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
