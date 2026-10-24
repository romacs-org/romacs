use super::*;

defsubr! { name = "define-charset-internal", gnu = "charset.c:838", effect = Global,
fn define_charset_internal(cx, name: Value, dimension: Value, space: Value, min: Value, max: Value, final_char: Value, revision: Value, mule: Value, ascii: Value, supplementary: Value, invalid: Value, offset: Value, map: Value, subset: Value, superset: Value, unify: Value, plist: Value; rest extra) {
    if !extra.is_empty() {
        let symbol = cx.intern("define-charset-internal");
        let count = Value::integer((17 + extra.len()) as i64).unwrap();
        let data = cx.cons(symbol, count);
        let condition = cx.intern("wrong-number-of-arguments");
        return Err(Fault::Signal(condition, data));
    }
    cx.charset_define(&[name, dimension, space, min, max, final_char, revision, mule, ascii, supplementary, invalid, offset, map, subset, superset, unify, plist])
}}
