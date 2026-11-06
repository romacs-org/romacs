//! Numeric comparisons from GNU `src/data.c` (`arithcompare`, data.c:2718).
//!
//! GNU coerces a marker to its position, then compares a fixnum and a float
//! exactly: the fixnum is converted to a double, and a floating tie is broken
//! by comparing the integer values, so `(<= most-positive-fixnum 2.0e18...)`
//! never rounds a distinct integer into equality. NaN compares false.
//! `arithcompare_driver` (data.c:2856) stops at the first failing pair, so an
//! argument after it is never type-checked, and a single argument is not
//! checked at all.
use super::*;
use crate::prim::Fault;

const LT: u8 = 1;
const GT: u8 = 2;
const EQ: u8 = 4;

enum Num {
    Int(num_bigint::BigInt),
    Float(f64),
}

/// GNU `coerce_marker` followed by the type dispatch of `arithcompare`, which
/// signals `(wrong-type-argument number-or-marker-p X)` via
/// `not_number_or_marker` (data.c:2711).
fn number(cx: &Leaf<'_>, v: Value) -> std::result::Result<Num, Fault> {
    if let Some(n) = cx.bigint_value(v) {
        return Ok(Num::Int(n));
    }
    if let Some(position) = cx.marker_position(v) {
        return position.map(|n| Num::Int(n.into())).ok_or(Fault::Error("Marker does not point anywhere".into()));
    }
    cx.float_value(v).map(Num::Float).map_err(|_| Fault::Wrong("number-or-marker-p", v))
}

fn bits(lt: bool, gt: bool, eq: bool) -> u8 {
    u8::from(lt) * LT | u8::from(gt) * GT | u8::from(eq) * EQ
}

/// Compare a float and an arbitrary integer without rounding the integer.
fn float_int(f: f64, i: num_bigint::BigInt) -> u8 {
    let order = crate::numbers::int_float_cmp(&i, f);
    let less = order == Some(std::cmp::Ordering::Greater);
    let greater = order == Some(std::cmp::Ordering::Less);
    bits(
        less,
        greater,
        order == Some(std::cmp::Ordering::Equal),
    )
}

fn order_bits(order: Option<std::cmp::Ordering>) -> u8 {
    match order {
        Some(std::cmp::Ordering::Less) => LT,
        Some(std::cmp::Ordering::Greater) => GT,
        Some(std::cmp::Ordering::Equal) => EQ,
        None => 0,
    }
}

/// GNU coerce_marker's failure; data.c arithcompare meets it for either
/// operand before it type-checks the first.
fn unset_marker(cx: &Leaf<'_>, v: Value) -> std::result::Result<(), Fault> {
    match cx.marker_position(v) {
        Some(None) => Err(Fault::Error("Marker does not point anywhere".into())),
        _ => Ok(()),
    }
}

fn arithcompare(cx: &Leaf<'_>, a: Value, b: Value) -> std::result::Result<u8, Fault> {
    if let Some(order) = cx.quick_compare(a, b) {
        return Ok(order_bits(order));
    }
    unset_marker(cx, a)?;
    unset_marker(cx, b)?;
    let x = number(cx, a)?;
    let y = number(cx, b)?;
    Ok(match (x, y) {
        (Num::Int(x), Num::Int(y)) => bits(x < y, x > y, x == y),
        (Num::Float(x), Num::Float(y)) => bits(x < y, x > y, x == y),
        (Num::Float(x), Num::Int(y)) => float_int(x, y),
        (Num::Int(x), Num::Float(y)) => {
            let r = float_int(y, x);
            // Swap the roles of LT and GT for the converse comparison.
            (r & EQ) | if r & LT != 0 { GT } else { 0 } | if r & GT != 0 { LT } else { 0 }
        }
    })
}

fn driver(cx: &Leaf<'_>, first: Value, rest: &[Value], mask: u8) -> Out {
    let mut previous = first;
    for &next in rest {
        if arithcompare(cx, previous, next)? & mask == 0 {
            return Ok(Value::NIL);
        }
        previous = next;
    }
    Ok(Value::TRUE)
}

defsubr! { name = "car-less-than-car", gnu = "fileio.c:5870", effect = Reads,
fn car_less_than_car(cx, a: Value, b: Value) {
    let car = |v| if v == Value::NIL { Ok(Value::NIL) }
        else { cx.pair(v).map(|p| p.0).ok_or(Fault::Wrong("listp", v)) };
    Ok(if arithcompare(cx, car(a)?, car(b)?)? & LT != 0 { Value::TRUE } else { Value::NIL })
}}

#[cfg(test)]
mod car_less_tests {
    #[test]
    fn compares_cars_numerically() {
        let mut vm = crate::Vm::new();
        let v = vm.eval_source_raw("(list (car-less-than-car '(1 x) '(2 y)) (car-less-than-car '(2) '(1)) (car-less-than-car '(1.5) '(2)) (condition-case e (car-less-than-car 3 '(4)) (error e)))").unwrap();
        assert_eq!(vm.display(v).unwrap(), "(t nil t (wrong-type-argument listp 3))");
    }
}

defsubr! { name = "mod", gnu = "data.c:3448", effect = Reads,
fn modulo(cx, x: Value, y: Value) {
    match (number(cx, x)?, number(cx, y)?) {
        (Num::Int(a), Num::Int(b)) => {
            if b == num_bigint::BigInt::from(0) {
                return Err(Fault::Signal(cx.intern("arith-error"), Value::NIL));
            }
            let mut remainder = a % &b;
            if remainder != num_bigint::BigInt::from(0) && remainder.sign() != b.sign() {
                remainder += b;
            }
            Ok(cx.bigint(remainder))
        }
        (a, b) => {
            let float = |n: Num| match n {
                Num::Float(n) => n,
                Num::Int(n) => n.to_string().parse::<f64>().expect("decimal integer is a float"),
            };
            let (a, b) = (float(a), float(b));
            let mut remainder = a % b;
            if if b < 0.0 { remainder > 0.0 } else { remainder < 0.0 } {
                remainder += b;
            }
            Ok(cx.float(remainder))
        }
    }
}}

defsubr! { name = "<=", gnu = "data.c:2894", effect = Pure,
fn leq(cx, first: Value; rest rest) {
    driver(cx, first, rest, LT | EQ)
}}

defsubr! { name = ">", gnu = "data.c:2883", effect = Pure,
fn gtr(cx, first: Value; rest rest) {
    driver(cx, first, rest, GT)
}}

defsubr! { name = ">=", gnu = "data.c:2905", effect = Pure,
fn geq(cx, first: Value; rest rest) {
    driver(cx, first, rest, GT | EQ)
}}

// Named `/=`, which primh cannot use as a file name; recorded through
// tools/primh/primh_filenames.py, which maps it to `%2F=`.
defsubr! { name = "/=", gnu = "data.c:2916", effect = Pure,
fn neq(cx, num1: Value, num2: Value) {
    Ok(if arithcompare(cx, num1, num2)? & EQ != 0 { Value::NIL } else { Value::TRUE })
}}
