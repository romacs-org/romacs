//! The five measured warm-chain stops, following GNU at the pin
//! (`gnu-emacs-ref` 45b0f7699df): alloc.c `make-vector`, fns.c `vconcat`,
//! `reverse` and `secure-hash-algorithms`, editfns.c `propertize`.
//! Verified by primh (`tools/primh/frontier5-specs`) and by value in
//! `tests/frontier5.rs`. Every body is a T1 leaf: it reads its arguments into
//! owned Rust data and allocates its one result; nothing collects in between.
use super::*;
use crate::lstring::char_string;
use std::result::Result;

// `memory-signal-data' (alloc.c:7508): `MEMORY_FULL' in gc.rs; VM2 has no such
// variable, so GNU's startup.el:1738 rewrite is not observable (VM2-FR5-2).
use crate::gc::MEMORY_FULL;

/// GNU `FOR_EACH_TAIL` (lisp.h:5839) then `CHECK_LIST_END (tail, tail)`: the
/// elements of LIST, `(wrong-type-argument listp TERMINATOR)` for a dotted
/// list, and `(circular-list TAIL)` naming the tail at which Brent's check
/// fired (the tortoise teleports at power-of-two multiples of 4096 steps).
fn list_elements(cx: &Leaf<'_>, list: Value) -> Result<Vec<Value>, Fault> {
    let mut out = Vec::new();
    let mut tail = list;
    let mut tortoise = list;
    let mut steps = 0u64;
    while let Some((car, cdr)) = cx.pair(tail) {
        out.push(car);
        tail = cdr;
        if tail == tortoise { return Err(Fault::Circular(tail)); }
        steps += 1;
        if steps & 4095 == 0 && steps.is_power_of_two() { tortoise = tail; }
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", tail)); }
    Ok(out)
}

/// Character codes of STRING as fns.c reads them: SREF bytes for unibyte,
/// decoded characters (raw bytes as #x3FFF80..) for multibyte.
fn string_codes(cx: &Leaf<'_>, string: Value) -> Result<Vec<Value>, Fault> {
    Ok(cx.string_view(string)?.chars().map(|c| Value::integer(i64::from(c)).unwrap()).collect())
}

fn allocate_vector(cx: &mut Leaf<'_>, values: Vec<Value>) -> Value {
    cx.vector_from(values)
}

defsubr! { name = "make-vector", gnu = "alloc.c:3530", effect = Pure,
fn make_vector(cx, length: Value, init: Value) {
    // CHECK_TYPE (FIXNATP (length) && XFIXNAT (length) <= PTRDIFF_MAX, Qwholenump, length)
    let n = length.integer_value().and_then(|n| usize::try_from(n).ok())
        .ok_or(Fault::Wrong("wholenump", length))?;
    // allocate_vectorlike: beyond VECTOR_ELTS_MAX, or a failed malloc, is
    // memory_full (alloc.c:4130), which signals `memory-signal-data`.
    let mut values = Vec::new();
    values.try_reserve_exact(n).map_err(|_| Fault::Error(MEMORY_FULL.into()))?;
    values.resize(n, init);
    Ok(allocate_vector(cx, values))
}}

defsubr! { name = "vconcat", gnu = "fns.c:750", effect = Pure,
fn vconcat(cx; rest sequences) {
    // concat_to_vector (fns.c:1114): every argument's type and length is
    // checked, in order, before the result exists.
    let mut values = Vec::new();
    for &arg in sequences {
        if arg == Value::NIL { continue; }
        if cx.pair(arg).is_some() {
            values.extend(list_elements(cx, arg)?);
        } else if let Ok(items) = cx.vector_values(arg) {
            values.extend(items);
        } else if let Ok(bits) = cx.bool_vector(arg) {
            values.extend((0..bits.len).map(|i| if bits.get(i).unwrap() { Value::TRUE } else { Value::NIL }));
        } else if cx.string_view(arg).is_ok() {
            values.extend(string_codes(cx, arg)?);
        } else {
            // GNU also accepts closures; VM2 closures have no GNU slots.
            return Err(Fault::Wrong("sequencep", arg));
        }
    }
    Ok(allocate_vector(cx, values))
}}

defsubr! { name = "propertize", gnu = "editfns.c:3296", effect = Pure,
fn propertize(cx, string: Value; rest properties) {
    // Number of args must be odd; the data names the SYMBOL (editfns.c:3309).
    let nargs = 1 + properties.len();
    if nargs % 2 == 0 {
        let name = cx.intern("propertize");
        return Err(Fault::WrongNumberOfArguments(name, Value::integer(nargs as i64).unwrap()));
    }
    cx.string_view(string)?;
    // properties = (PN VN ... P1 V1); add_properties walks that list, so
    // later pairs are added first and an earlier duplicate's value wins.
    let mut keyed = Vec::with_capacity(properties.len() / 2);
    for pair in properties.chunks_exact(2).rev() {
        keyed.push((property_key(cx, pair[0])?, pair[1]));
    }
    cx.propertized_copy(string, &keyed)
}}

/// A property is keyed by its symbol's identity. GNU accepts any object
/// compared with `eq`; an object that is no symbol is refused (VM2-FR5-3).
/// The refusal is a plain `error` carrying the key: no type contract is
/// invented for an argument GNU accepts.
fn property_key(cx: &mut Leaf<'_>, key: Value) -> Result<crate::properties::Key, Fault> {
    crate::properties::Key::of(key).ok_or_else(|| {
        let message = cx.string("VM2 string properties need a symbol key");
        let data = cx.cons(key, Value::NIL);
        let data = cx.cons(message, data);
        let error = cx.intern("error");
        Fault::Signal(error, data)
    })
}

defsubr! { name = "secure-hash-algorithms", gnu = "fns.c:6092", effect = Pure,
fn secure_hash_algorithms(cx) {
    let mut list = Value::NIL;
    for name in ["md5", "sha1", "sha224", "sha256", "sha384", "sha512",
                 "sha3-224", "sha3-256", "sha3-384", "sha3-512"].into_iter().rev() {
        let symbol = cx.intern(name);
        list = cx.cons(symbol, list);
    }
    Ok(list)
}}
