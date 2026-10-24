//! Small GNU scalar primitives, sharing only existing numeric/marker substrates.
use super::*;

// GNU XFLOATINT uses mpz_get_d_rounded: decimal rendering followed by
// correctly rounded strtod (bignum.c:412), including overflow to infinity.
fn number(cx: &Leaf<'_>, value: Value) -> std::result::Result<f64, Fault> {
    if let Some(n) = value.integer_value() {
        return Ok(n as f64);
    }
    if let Some(n) = cx.bigint_value(value) {
        return Ok(n
            .to_string()
            .parse::<f64>()
            .expect("decimal integer is a float"));
    }
    cx.float_value(value)
        .map_err(|_| Fault::Wrong("numberp", value))
}

defsubr! { name = "atan", gnu = "floatfns.c:103", effect = Pure,
fn atan(cx, y: Value; optional x: Value) {
    let y = number(cx, y)?;
    let result = if x == Value::NIL { y.atan() } else { y.atan2(number(cx, x)?) };
    Ok(cx.float(result))
}}

defsubr! { name = "exp", gnu = "floatfns.c:210", effect = Pure,
fn exp(cx, arg: Value) {
    let result = number(cx, arg)?.exp();
    Ok(cx.float(result))
}}

defsubr! { name = "number-to-string", gnu = "data.c:3067", effect = Reads,
fn number_to_string(cx, arg: Value) {
    let text = if let Some(n) = cx.bigint_value(arg) { n.to_string() } else {
        let n = cx.float_value(arg).map_err(|_| Fault::Wrong("numberp", arg))?;
        let symbol = cx.intern("float-output-format");
        let format = cx.variable_value(symbol)?.and_then(|v| cx.string_text(v).ok());
        float_text(n, format.as_deref())
    };
    Ok(cx.make_string(text.into_bytes(), false))
}}

fn scientific(n: f64, precision: usize) -> String {
    let raw = format!("{n:.precision$e}");
    let (mantissa, exponent) = raw.split_once('e').expect("Rust scientific format");
    let exponent: i32 = exponent.parse().expect("Rust exponent");
    format!("{mantissa}e{exponent:+03}")
}

fn general(n: f64, precision: usize) -> String {
    let raw = scientific(n, precision - 1);
    let (mantissa, exponent) = raw.split_once('e').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    if exponent < -4 || exponent >= precision as i32 {
        let mantissa = if mantissa.contains('.') {
            mantissa.trim_end_matches('0').trim_end_matches('.')
        } else {
            mantissa
        };
        format!("{mantissa}e{exponent:+03}")
    } else {
        let places = (precision as i32 - exponent - 1).max(0) as usize;
        let raw = format!("{n:.places$}");
        if raw.contains('.') {
            raw.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            raw
        }
    }
}

fn float_text(n: f64, custom: Option<&str>) -> String {
    if n.is_nan() {
        let payload = n.to_bits() & 0x0007_ffff_ffff_ffff;
        return format!(
            "{}{payload}.0e+NaN",
            if n.is_sign_negative() { "-" } else { "" }
        );
    }
    if n.is_infinite() {
        return crate::numbers::print_float(n);
    }
    if let Some(format) = custom
        .map(|s| s.split('\0').next().unwrap())
        .and_then(|s| s.strip_prefix("%."))
    {
        let (digits, mode) = format.split_at(format.char_indices().last().map_or(0, |(i, _)| i));
        if digits.bytes().all(|b| b.is_ascii_digit()) && matches!(mode, "f" | "e" | "g") {
            if let Ok(p) = if digits.is_empty() {
                Ok(0)
            } else {
                digits.parse::<usize>()
            } {
                if p <= 15 && (p != 0 || mode == "f" || digits.is_empty()) {
                    let mut out = match mode {
                        "f" => format!("{n:.p$}"),
                        "e" => scientific(n, p),
                        _ => general(n, p.max(1)),
                    };
                    if !(p == 0 && mode == "f" && !digits.is_empty()) && !out.contains(['.', 'e']) {
                        out.push_str(".0");
                    }
                    return out;
                }
            }
        }
    }
    // GNU dtoastr starts at one significant digit for subnormals, DBL_DIG
    // otherwise, increasing until strtod round-trips (lib/ftoastr.c:144).
    for p in if n.is_subnormal() { 1 } else { 15 }..=17 {
        let mut out = general(n, p);
        if out.parse::<f64>().is_ok_and(|v| v.to_bits() == n.to_bits()) {
            if !out.contains(['.', 'e']) {
                out.push_str(".0");
            }
            return out;
        }
    }
    unreachable!("17 digits round-trip binary64")
}
