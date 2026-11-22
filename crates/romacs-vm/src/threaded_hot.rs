//! The hot handlers, about 85% of the ops a Lisp benchmark runs, as safe
//! Rust over the audited core's capabilities (`threaded_audited.rs`,
//! lisp-speed.md §13.6). A window is a `[Value; 257]` indexed by byte
//! operands, or one past a byte for a kernel's second argument, so no bounds
//! check is emitted; a frame whose operands do not
//! fit a byte takes the `_wide` handler, through the validated slot
//! (`Regs::slot`). Each handler ends in one dispatch in tail position, with
//! no `Drop` or address-taken local across it, so it compiles to a jump
//! (`tools/check_threading.py` checks).
#![forbid(unsafe_code)]
use super::audited::{go, legacy, poll, Branch, Bytes, Hot, Kernel, Pc, Regs, Wides};
use super::Value;

macro_rules! truth {
    ($c:expr) => {
        if $c { Value::TRUE } else { Value::NIL }
    };
}

#[inline]
pub(super) fn t_mov<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Bytes>) -> Value {
    let regs = r.window(&mut e);
    regs[pc.a()] = regs[pc.b()];
    go(e, r, pc.next())
}
#[inline]
pub(super) fn t_mov_wide<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Wides>) -> Value {
    let value = *r.slot(&mut e, pc.wide_b());
    *r.slot(&mut e, pc.wide_a()) = value;
    go(e, r, pc.next())
}
#[inline]
pub(super) fn t_constant<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Bytes>) -> Value {
    r.window(&mut e)[pc.a()] = pc.value();
    go(e, r, pc.next())
}
#[inline]
pub(super) fn t_constant_wide<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Wides>) -> Value {
    *r.slot(&mut e, pc.wide_a()) = pc.value();
    go(e, r, pc.next())
}
#[inline]
pub(super) fn t_unless<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Bytes>) -> Value {
    let to = if r.window(&mut e)[pc.a()] == Value::NIL { pc.branch() } else { pc.next() };
    go(e, r, to)
}
#[inline]
pub(super) fn t_unless_wide<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Wides>) -> Value {
    let to = if *r.slot(&mut e, pc.wide_a()) == Value::NIL { pc.branch() } else { pc.next() };
    go(e, r, to)
}
/// A backward jump spends the engine's budget, and polls when it is gone.
#[inline]
pub(super) fn t_jump<'f>(mut e: Hot<'f, '_, '_>, r: Regs<'f>, pc: Pc<'f, Branch>) -> Value {
    let to = pc.branch();
    if pc.is_back(&to) && e.spend() {
        return poll(e, r, to);
    }
    go(e, r, to)
}

/// `car' or `cdr' of a private cons or nil; the kernel takes the rest.
macro_rules! half {
    ($name:ident, $field:tt) => {
        #[inline]
        pub(super) fn $name<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Kernel>) -> Value {
            let x = r.window(&mut e)[pc.d()];
            if x.0 & 7 == 3 {
                if let Some(pair) = e.pair((x.0 >> 3) as usize) {
                    let value = pair.$field;
                    r.window(&mut e)[pc.a()] = value;
                    return go(e, r, pc.next());
                }
            } else if x == Value::NIL {
                r.window(&mut e)[pc.a()] = Value::NIL;
                return go(e, r, pc.next());
            }
            legacy(e, r, pc)
        }
    };
}
half!(t_car, 0);
half!(t_cdr, 1);

/// A kernel's fixnum case: X at D, Y at D+1 (or 1 for the unary ones);
/// anything else, an overflow included, is the kernel's.
macro_rules! fixnums {
    ($name:ident, $y:expr, |$x:ident, $yv:ident| $body:expr) => {
        #[inline]
        pub(super) fn $name<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Kernel>) -> Value {
            let regs = r.window(&mut e);
            let $x = regs[pc.d()];
            let $yv: Value = $y(regs, pc);
            // Both tags at once: a fixnum less one has clear low bits.
            if ($x.0.wrapping_sub(1) | $yv.0.wrapping_sub(1)) & 3 != 0 {
                return legacy(e, r, pc);
            }
            let result: Option<Value> = $body;
            match result {
                Some(v) => {
                    regs[pc.a()] = v;
                    go(e, r, pc.next())
                }
                None => legacy(e, r, pc),
            }
        }
    };
}
#[inline]
fn second(regs: &[Value; 257], pc: Pc<'_, Kernel>) -> Value {
    regs[pc.d() + 1]
}
#[inline]
fn one(_: &[Value; 257], _: Pc<'_, Kernel>) -> Value {
    Value(5)
}
fixnums!(t_add, second, |x, y| x.0.cast_signed().checked_add((y.0 - 1).cast_signed()).map(|n| Value(n.cast_unsigned())));
fixnums!(t_sub, second, |x, y| x.0.cast_signed().checked_sub((y.0 - 1).cast_signed()).map(|n| Value(n.cast_unsigned())));
fixnums!(t_inc, one, |x, _y| x.0.cast_signed().checked_add(4).map(|n| Value(n.cast_unsigned())));
fixnums!(t_dec, one, |x, _y| x.0.cast_signed().checked_sub(4).map(|n| Value(n.cast_unsigned())));
fixnums!(t_less, second, |x, y| Some(truth!(x.0.cast_signed() < y.0.cast_signed())));
fixnums!(t_greater, second, |x, y| Some(truth!(x.0.cast_signed() > y.0.cast_signed())));
fixnums!(t_equal, second, |x, y| Some(truth!(x == y)));

#[inline]
pub(super) fn t_null<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Kernel>) -> Value {
    let regs = r.window(&mut e);
    regs[pc.a()] = truth!(regs[pc.d()] == Value::NIL);
    go(e, r, pc.next())
}
#[inline]
pub(super) fn t_consp<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Kernel>) -> Value {
    let regs = r.window(&mut e);
    regs[pc.a()] = truth!(regs[pc.d()].0 & 7 == 3);
    go(e, r, pc.next())
}
#[inline]
pub(super) fn t_eq<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Kernel>) -> Value {
    let regs = r.window(&mut e);
    regs[pc.a()] = truth!(regs[pc.d()] == regs[pc.d() + 1]);
    go(e, r, pc.next())
}
