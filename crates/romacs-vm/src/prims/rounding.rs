//! `floor`, `round`, `ceiling` and `truncate`, GNU `rounding_driver` (floatfns.c:404),
//! measured on the window journey: GNU's own `split-window` rounds a new
//! window's pixel size to a character multiple with `round`
//! (window.el:5846) and `window--min-size-1` uses `ceiling`.
//!
//! GNU divides exactly: a float operand is scaled to an integer
//! (`double_integer_scale`, `rescale_for_division`) and the quotient is an
//! mpz division, so `(round 1.5 0.5)` and `(ceiling 0.1 0.01)` see the exact
//! binary values, never a rounded float quotient. VM2 does the same with each
//! double as an odd integer times a power of two, in big integers; fixnums
//! divide directly.
use super::*;
use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use std::result::Result;

enum Num {
    Int(i64),
    Big(BigInt),
    Float(f64),
}

/// `CHECK_NUMBER`: an integer or a float, never a marker.
fn number(cx: &Leaf<'_>, v: Value) -> Result<Num, Fault> {
    if let Some(n) = v.integer_value() {
        return Ok(Num::Int(n));
    }
    if cx.kind(v) == crate::prim::leaf::Kind::Bignum {
        return cx.bigint_value(v).map(Num::Big).ok_or(Fault::Wrong("numberp", v));
    }
    cx.float_value(v).map(Num::Float).map_err(|_| Fault::Wrong("numberp", v))
}

fn signal(cx: &mut Leaf<'_>, condition: &str) -> Fault {
    let condition = cx.intern(condition);
    Fault::Signal(condition, Value::NIL)
}

/// make_integer: a fixnum, or a bignum past the fixnum range.
fn integer(cx: &mut Leaf<'_>, n: i128) -> Out {
    Ok(cx.bigint(num_bigint::BigInt::from(n)))
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Floor,
    Ceiling,
    Round,
    Truncate,
}

/// A finite double as (odd-or-zero mantissa, exponent): D = M * 2^E.
fn decompose(d: f64) -> (BigInt, i32) {
    if d == 0.0 {
        return (BigInt::from(0), 0);
    }
    let bits = d.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = (bits & ((1 << 52) - 1)) as i64;
    let (mut m, mut e) = if exponent == 0 { (fraction, -1074) } else { (fraction | (1 << 52), exponent - 1075) };
    while m & 1 == 0 {
        m >>= 1;
        e += 1;
    }
    (BigInt::from(if d < 0.0 { -m } else { m }), e)
}

/// The exact quotient of (NM * 2^NE) / (DM * 2^DE), rounded by MODE:
/// floor2, ceiling2 and round2 (floatfns.c:448, :466) on Q, truncated toward
/// zero, and its remainder R.
fn exact(cx: &mut Leaf<'_>, mode: Mode, (nm, ne): (BigInt, i32), (dm, de): (BigInt, i32)) -> Out {
    let (n, d) = (nm << (ne - de).max(0) as usize, dm << (de - ne).max(0) as usize);
    let (q, r) = n.div_rem(&d);
    let negative = (n.sign() == Sign::Minus) != (d.sign() == Sign::Minus);
    let exact = r.sign() == Sign::NoSign;
    let away = match mode {
        Mode::Truncate => false,
        Mode::Floor => !exact && negative,
        Mode::Ceiling => !exact && !negative,
        Mode::Round => {
            let twice = r.magnitude() * 2u32;
            match twice.cmp(d.magnitude()) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => q.bit(0),
                std::cmp::Ordering::Less => false,
            }
        }
    };
    let step = BigInt::from(if negative { -1 } else { 1 });
    Ok(cx.bigint(if away { q + step } else { q }))
}

/// GNU `double_integer_scale` > DBL_MANT_DIG - DBL_MIN_EXP, i.e. not finite.
fn finite(cx: &mut Leaf<'_>, x: f64) -> Result<(), Fault> {
    if x.is_finite() { Ok(()) } else { Err(signal(cx, "overflow-error")) }
}

fn driver(cx: &mut Leaf<'_>, arg: Value, divisor: Value, mode: Mode) -> Out {
    let n = number(cx, arg)?;
    if divisor == Value::NIL {
        return match n {
            Num::Int(_) => Ok(arg),
            Num::Float(x) => {
                // emacs_rint is rint (ties to even); ceiling is ceil. Then
                // double_to_integer: a non-finite value is overflow-error.
                finite(cx, x)?;
                let y = match mode {
                    Mode::Round => x.round_ties_even(),
                    Mode::Floor => x.floor(),
                    Mode::Ceiling => x.ceil(),
                    Mode::Truncate => x.trunc(),
                };
                // An integral double is M * 2^E exactly.
                let (n, e) = decompose(y);
                Ok(cx.bigint(if e >= 0 { n << e as u32 } else { n >> (-e) as u32 }))
            }
            Num::Big(_) => Ok(arg),
        };
    }
    let d = number(cx, divisor)?;
    match (&n, &d) {
        (_, Num::Int(0)) => return Err(signal(cx, "arith-error")),
        (_, Num::Float(y)) if *y == 0.0 => return Err(signal(cx, "arith-error")),
        _ => {}
    }
    match (n, d) {
        (Num::Int(n), Num::Int(d)) => {
            // round2 / ceiling2 on fixnums (floatfns.c:448, :466).
            let (n, d) = (i128::from(n), i128::from(d));
            let (q, r) = (n / d, n % d);
            let q = match mode {
                Mode::Truncate => q,
                Mode::Floor => q - i128::from(r != 0 && (n < 0) != (d < 0)),
                Mode::Ceiling => q + i128::from(r != 0 && (n < 0) == (d < 0)),
                Mode::Round => {
                    let (abs_r, abs_r1) = (r.abs(), d.abs() - r.abs());
                    if abs_r1 < abs_r + (q & 1) { q + if (d < 0) == (r < 0) { 1 } else { -1 } } else { q }
                }
            };
            integer(cx, q)
        }
        (n, d) => {
            // A non-finite float operand: an infinite divisor under a finite
            // numerator gives 0; every other case cannot be rescaled and is
            // overflow-error (floatfns.c:419-428, 381).
            let nf = match n { Num::Float(x) => Some(x), _ => None };
            if let Num::Float(y) = d {
                if y.is_infinite() && nf.is_none_or(f64::is_finite) {
                    return Ok(Value::integer(0).unwrap());
                }
                finite(cx, y)?;
            }
            if let Some(x) = nf {
                finite(cx, x)?;
            }
            let part = |v: Num| match v {
                Num::Int(i) => (BigInt::from(i), 0),
                Num::Big(i) => (i, 0),
                Num::Float(x) => decompose(x),
            };
            exact(cx, mode, part(n), part(d))
        }
    }
}

defsubr! { name = "ceiling", gnu = "floatfns.c:524", effect = Pure,
fn ceiling(cx, arg: Value; optional divisor: Value) {
    driver(cx, arg, divisor, Mode::Ceiling)
}}

defsubr! { name = "round", gnu = "floatfns.c:542", effect = Pure,
fn round(cx, arg: Value; optional divisor: Value) {
    driver(cx, arg, divisor, Mode::Round)
}}

defsubr! { name = "truncate", gnu = "floatfns.c:559", effect = Pure,
fn truncate(cx, arg: Value; optional divisor: Value) {
    driver(cx, arg, divisor, Mode::Truncate)
}}

defsubr! { name = "floor", gnu = "floatfns.c:530", effect = Pure,
fn floor(cx, arg: Value; optional divisor: Value) {
    driver(cx, arg, divisor, Mode::Floor)
}}
