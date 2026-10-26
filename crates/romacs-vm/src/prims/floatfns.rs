//! The rest of GNU floatfns.c: libm functions over `extract_float`, the
//! float-only rounding functions, and `expt`.
use super::leaf_scalars::number;
use super::*;

fn libm(cx: &mut Leaf<'_>, arg: Value, f: fn(f64) -> f64) -> Out {
    let x = number(cx, arg)?;
    Ok(cx.float(f(x)))
}

defsubr! { name = "acos", gnu = "floatfns.c:85", effect = Pure,
fn acos(cx, arg: Value) { libm(cx, arg, f64::acos) }}

defsubr! { name = "asin", gnu = "floatfns.c:94", effect = Pure,
fn asin(cx, arg: Value) { libm(cx, arg, f64::asin) }}

defsubr! { name = "cos", gnu = "floatfns.c:123", effect = Pure,
fn cos(cx, arg: Value) { libm(cx, arg, f64::cos) }}

defsubr! { name = "sin", gnu = "floatfns.c:132", effect = Pure,
fn sin(cx, arg: Value) { libm(cx, arg, f64::sin) }}

defsubr! { name = "tan", gnu = "floatfns.c:141", effect = Pure,
fn tan(cx, arg: Value) { libm(cx, arg, f64::tan) }}

defsubr! { name = "sqrt", gnu = "floatfns.c:259", effect = Pure,
fn sqrt(cx, arg: Value) { libm(cx, arg, f64::sqrt) }}

defsubr! { name = "isnan", gnu = "floatfns.c:150", effect = Pure,
fn isnan(cx, x: Value) {
    Ok(if cx.float_value(x)?.is_nan() { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "copysign", gnu = "floatfns.c:164", effect = Pure,
fn copysign(cx, x1: Value, x2: Value) {
    let (f1, f2) = (cx.float_value(x1)?, cx.float_value(x2)?);
    // X1 itself when the signs already agree, as GNU avoids make_float.
    Ok(if f1.is_sign_negative() != f2.is_sign_negative() { cx.float(-f1) } else { x1 })
}}

/// C frexp: X = SIGNIFICAND * 2^EXPONENT with |SIGNIFICAND| in [0.5, 1).
fn frexp_parts(x: f64) -> (f64, i64) {
    if x == 0.0 || !x.is_finite() { return (x, 0); }
    let (x, scale) = if x.abs() < f64::MIN_POSITIVE { (x * 2f64.powi(54), -54) } else { (x, 0) };
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i64 - 1022;
    (f64::from_bits(bits & !(0x7ff << 52) | (1022 << 52)), exponent + scale)
}

/// C ldexp (musl scalbn): X * 2^N, rounded once.
fn ldexp_value(mut x: f64, mut n: i64) -> f64 {
    let (max, min) = (f64::from_bits(0x7fe0_0000_0000_0000), f64::from_bits(0x0010_0000_0000_0000) * 2f64.powi(53));
    for _ in 0..2 {
        if n > 1023 { x *= max; n -= 1023; }
        else if n < -1022 { x *= min; n += 1022 - 53; }
    }
    let n = n.clamp(-1022, 1023);
    x * f64::from_bits(((0x3ff + n) as u64) << 52)
}

defsubr! { name = "frexp", gnu = "floatfns.c:182", effect = Pure,
fn frexp(cx, x: Value) {
    let (significand, exponent) = frexp_parts(number(cx, x)?);
    let significand = cx.float(significand);
    Ok(cx.cons(significand, Value::integer(exponent).unwrap()))
}}

defsubr! { name = "ldexp", gnu = "floatfns.c:200", effect = Pure,
fn ldexp(cx, sgnfcand: Value, exponent: Value) {
    let e = exponent.integer_value().ok_or(Fault::Wrong("fixnump", exponent))?
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    let x = number(cx, sgnfcand)?;
    Ok(cx.float(ldexp_value(x, e)))
}}

/// bignum.c expt_integer: an integer power, overflow-error past
/// integer-width bits (decided before computing, from a lower bound).
fn expt_integer(cx: &mut Leaf<'_>, base: num_bigint::BigInt, exponent: num_bigint::BigInt) -> Out {
    use num_bigint::BigInt;
    let (one, minus_one) = (BigInt::from(1), BigInt::from(-1));
    if base == one { return Ok(cx.bigint(base)); }
    if base == BigInt::from(0) {
        return Ok(Value::integer(i64::from(exponent == BigInt::from(0))).unwrap());
    }
    if base == minus_one {
        let odd = (&exponent % 2u32) != BigInt::from(0);
        return Ok(cx.bigint(if odd { minus_one } else { one }));
    }
    let symbol = cx.intern("overflow-error");
    let overflow = || Fault::Signal(symbol, Value::NIL);
    let e = u64::try_from(&exponent).ok().filter(|&e| {
        (base.bits() - 1).saturating_mul(e).saturating_add(1) <= super::bitwise::INTEGER_WIDTH
    });
    let Some(e) = e else { return Err(overflow()) };
    let result = base.pow(e as u32);
    if result.bits() > super::bitwise::INTEGER_WIDTH { return Err(overflow()); }
    Ok(cx.bigint(result))
}

defsubr! { name = "expt", gnu = "floatfns.c:219", effect = Pure,
fn expt(cx, arg1: Value, arg2: Value) {
    let x = number(cx, arg1)?;
    let y = number(cx, arg2)?;
    if let (Some(base), Some(exponent)) = (cx.bigint_value(arg1), cx.bigint_value(arg2)) {
        if exponent >= num_bigint::BigInt::from(0) { return expt_integer(cx, base, exponent); }
    }
    Ok(cx.float(x.powf(y)))
}}

defsubr! { name = "float", gnu = "floatfns.c:296", effect = Pure,
fn float(cx, arg: Value) {
    let x = number(cx, arg)?;
    Ok(if cx.float_value(arg).is_ok() { arg } else { cx.float(x) })
}}

fn float_rounding(cx: &mut Leaf<'_>, arg: Value, f: fn(f64) -> f64) -> Out {
    let x = cx.float_value(arg)?;
    Ok(cx.float(f(x)))
}

defsubr! { name = "fceiling", gnu = "floatfns.c:584", effect = Pure,
fn fceiling(cx, arg: Value) { float_rounding(cx, arg, f64::ceil) }}

defsubr! { name = "ffloor", gnu = "floatfns.c:595", effect = Pure,
fn ffloor(cx, arg: Value) { float_rounding(cx, arg, f64::floor) }}

defsubr! { name = "fround", gnu = "floatfns.c:606", effect = Pure,
fn fround(cx, arg: Value) { float_rounding(cx, arg, f64::round_ties_even) }}

defsubr! { name = "ftruncate", gnu = "floatfns.c:616", effect = Pure,
fn ftruncate(cx, arg: Value) { float_rounding(cx, arg, f64::trunc) }}
