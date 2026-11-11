//! GNU data.c arithmetic driver (3199) and bitwise DEFUNs (3494ff).
use crate::prim::{Fault, Leaf};
use crate::Value;

fn integer_marker(cx: &Leaf<'_>, value: Value, first: bool) -> Result<num_bigint::BigInt, Fault> {
    if let Some(n) = cx.bigint_value(value) {
        return Ok(n);
    }
    if let Some(position) = cx.marker_position(value) {
        return position
            .map(Into::into)
            .ok_or_else(|| Fault::Error("Marker does not point anywhere".into()));
    }
    let predicate = if first || cx.float_value(value).is_ok() {
        "integer-or-marker-p"
    } else {
        "number-or-marker-p"
    };
    Err(Fault::Wrong(predicate, value))
}

/// ARGS folded by OP from IDENTITY when every one is a fixnum, as GNU's
/// `arith_driver` folds them in machine words before it reaches for a bignum.
/// The bitwise operations of two fixnums are fixnums.
#[inline]
fn fold_fixnums(args: &[Value], identity: i64, op: fn(i64, i64) -> i64) -> Option<Value> {
    let mut result = identity;
    for value in args {
        result = op(result, value.integer_value()?);
    }
    Value::fixnum(result)
}

defsubr! { name = "logior", gnu = "data.c:3506", effect = Reads,
    fn logior(cx; rest args) {
        if let Some(value) = fold_fixnums(args, 0, |a, b| a | b) { return Ok(value); }
        if args.len() == 1 && cx.bigint_value(args[0]).is_some() { return Ok(args[0]); }
        let mut result = num_bigint::BigInt::from(0);
        for (i, value) in args.iter().copied().enumerate() {
            result |= integer_marker(cx, value, i == 0)?;
        }
        Ok(cx.bigint(result))
    }
}

defsubr! { name = "logand", gnu = "data.c:3494", effect = Reads,
    fn logand(cx; rest args) {
        if let Some(value) = fold_fixnums(args, -1, |a, b| a & b) { return Ok(value); }
        if args.len() == 1 && cx.bigint_value(args[0]).is_some() { return Ok(args[0]); }
        let mut result = num_bigint::BigInt::from(-1);
        for (i, value) in args.iter().copied().enumerate() {
            result &= integer_marker(cx, value, i == 0)?;
        }
        Ok(cx.bigint(result))
    }
}

defsubr! { name = "logxor", gnu = "data.c:3518", effect = Reads,
    fn logxor(cx; rest args) {
        if let Some(value) = fold_fixnums(args, 0, |a, b| a ^ b) { return Ok(value); }
        if args.len() == 1 && cx.bigint_value(args[0]).is_some() { return Ok(args[0]); }
        let mut result = num_bigint::BigInt::from(0);
        for (i, value) in args.iter().copied().enumerate() {
            result ^= integer_marker(cx, value, i == 0)?;
        }
        Ok(cx.bigint(result))
    }
}

defsubr! { name = "lognot", gnu = "data.c:3684", effect = Reads,
    fn lognot(cx, number: Value) {
        if let Some(value) = number.integer_value().and_then(|n| Value::fixnum(!n)) { return Ok(value); }
        let n = cx.bigint_value(number).ok_or(Fault::Wrong("integerp", number))?;
        Ok(cx.bigint(!n))
    }
}

defsubr! { name = "logcount", gnu = "data.c:3530", effect = Pure,
    fn logcount(cx, value: Value) {
        if let Some(n) = value.integer_value() {
            let ones = if n < 0 { !n } else { n }.count_ones();
            return Ok(Value::fixnum(i64::from(ones)).expect("a count of bits is a fixnum"));
        }
        let n = cx.bigint_value(value).ok_or(Fault::Wrong("integerp", value))?;
        // A negative VALUE counts its zero bits: the ones of its complement.
        let nonnegative = if n.sign() == num_bigint::Sign::Minus { !n } else { n };
        let ones = nonnegative.magnitude().count_ones();
        Value::integer(ones as i64).map_err(|e| Fault::Error(e.into()))
    }
}

/// GNU integer-width's default (bignum.c): results wider than this overflow.
pub(super) const INTEGER_WIDTH: u64 = 65536;

defsubr! { name = "ash", gnu = "data.c:3555", effect = Reads,
    fn ash(cx, value: Value, count: Value) {
        if let (Some(n), Some(c)) = (value.integer_value(), count.integer_value()) {
            // A right shift of a fixnum is a fixnum; a left shift is one
            // when the wide result still fits.
            let shifted = if c <= 0 { Some(n >> c.unsigned_abs().min(63)) }
                else if c < 64 { i64::try_from(i128::from(n) << c).ok() } else { None };
            if let Some(value) = shifted.and_then(Value::fixnum) { return Ok(value); }
        }
        let n = cx.bigint_value(value).ok_or(Fault::Wrong("integerp", value))?;
        let c = cx.bigint_value(count).ok_or(Fault::Wrong("integerp", count))?;
        let symbol = cx.intern("overflow-error");
        let overflow = || Fault::Signal(symbol, Value::NIL);
        let zero = num_bigint::BigInt::from(0);
        if n == zero { return Ok(value); }
        let shift = match i64::try_from(&c) {
            Ok(shift) if shift.unsigned_abs() <= u64::from(u32::MAX) => shift,
            _ if c < zero => return Ok(cx.bigint(if n < zero { (-1).into() } else { zero })),
            _ => return Err(overflow()),
        };
        if shift >= 0 {
            if n.bits() + shift as u64 > INTEGER_WIDTH { return Err(overflow()); }
            Ok(cx.bigint(n << shift as usize))
        } else {
            // Arithmetic right shift rounds toward negative infinity.
            Ok(cx.bigint(n >> shift.unsigned_abs() as usize))
        }
    }
}
