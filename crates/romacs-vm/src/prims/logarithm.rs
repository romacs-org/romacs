use super::*;

fn extract_float(cx: &Leaf<'_>, value: Value) -> std::result::Result<f64, Fault> {
    if let Some(integer) = cx.bigint_value(value) {
        Ok(crate::numbers::integer_float(&integer))
    } else {
        cx.float_value(value).map_err(|_| Fault::Wrong("numberp", value))
    }
}

defsubr! { name = "log", gnu = "floatfns.c:234", effect = Pure,
fn logarithm(cx, argument: Value; optional base: Value) {
    let number = extract_float(cx, argument)?;
    let result = if base == Value::NIL { number.ln() } else {
        match extract_float(cx, base)? {
            10.0 => number.log10(),
            2.0 => number.log2(),
            base => number.ln() / base.ln(),
        }
    };
    Ok(cx.float(result))
}}
