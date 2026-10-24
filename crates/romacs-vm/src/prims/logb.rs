use super::*;

defsubr! { name = "logb", gnu = "floatfns.c:305", effect = Pure,
fn logb(cx, arg: Value) {
    if let Some(n) = arg.integer_value() {
        return if n == 0 {
            Ok(cx.float(f64::NEG_INFINITY))
        } else {
            Ok(Value::integer(i64::from(n.unsigned_abs().ilog2())).expect("integer exponent"))
        };
    }
    if let Some(n) = cx.bigint_value(arg) {
        return Ok(Value::integer((n.bits() - 1) as i64).expect("bignum exponent"));
    }
    let number = cx.float_value(arg).map_err(|_| Fault::Wrong("numberp", arg))?;
    if number == 0.0 {
        return Ok(cx.float(f64::NEG_INFINITY));
    }
    if !number.is_finite() {
        return Ok(if number < 0.0 { cx.float(-number) } else { arg });
    }
    let bits = number.to_bits();
    let biased_exponent = ((bits >> 52) & 0x7ff) as i64;
    let exponent = if biased_exponent == 0 {
        i64::from((bits & ((1_u64 << 52) - 1)).ilog2()) - 1074
    } else {
        biased_exponent - 1023
    };
    Ok(Value::integer(exponent).expect("float exponent"))
}}
