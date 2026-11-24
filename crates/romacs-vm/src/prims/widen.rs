//! Widened declaration proving primitives and evaluator regression witnesses.
use super::*;
use crate::quit::{KERNEL_CHUNK, POLL_STEPS};

defsubr! { name = "vectorp", gnu = "data.c:398", effect = Pure,
fn vectorp(cx, object: Value) {
    Ok(if cx.kind(object) == crate::prim::leaf::Kind::Vector { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "symbol-function", gnu = "data.c:815", effect = Reads,
fn symbol_function(cx, symbol: Value) {
    cx.symbol_definition(symbol)
}}

defsubr! { name = "use-global-map", gnu = "keymap.c:1895", effect = Global,
fn use_global_map(cx, map: Value) {
    cx.use_global_map(map)
}}

defsubr! { name = "make-symbol", gnu = "alloc.c:3708", effect = Pure,
fn make_symbol(cx, name: Value) {
    let key = cx.string_view(name).map_err(|_| Fault::Wrong("stringp", name)).map(|v| crate::obarray::symbol_key(v.bytes(), v.len()).map(std::borrow::Cow::into_owned))?;
    Ok(cx.make_symbol(&key.ok_or_else(|| Fault::NotUnicode(name))?))
}}

defsubr! { name = "listp", gnu = "data.c:332", effect = Pure,
fn listp(cx, object: Value) {
    Ok(if object == Value::NIL || cx.pair(object).is_some() { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "numberp", gnu = "data.c:601", effect = Pure,
fn numberp(cx, object: Value) {
    Ok(if matches!(cx.kind(object), crate::prim::leaf::Kind::Integer | crate::prim::leaf::Kind::Bignum | crate::prim::leaf::Kind::Float) { Value::TRUE } else { Value::NIL })
}}

// GNU data.c:826 checks SYMBOL before returning its name. VM2 currently
// materializes name text; symbol-name identity is tracked by primh explicitly.
defsubr! { name = "symbol-name", gnu = "data.c:826", effect = Pure,
fn symbol_name(cx, symbol: Value) {
    cx.symbol_name_value(symbol)
}}

// Default-obarray interning is sufficient for the empty-name identity witness.
defsubr! { name = "intern", gnu = "lread.c:4771", effect = Global,
fn intern(cx, name: Value; optional obarray: Value) {
    let table = cx.resolve_obarray(obarray)?;
    cx.intern_in(table, name)
}}

defsubr! { name = "concat", gnu = "fns.c:737", effect = Pure,
fn concat(cx; rest args) {
    if let Some(result) = cx.concat_ascii(args) { return Ok(result); }
    concat_fallback(cx, args)
}}

fn concat_fallback(cx: &mut Leaf<'_>, args: &[Value]) -> Out {
    let mut chunks = Vec::new();
    let mut parts = Vec::new();
    let mut multibyte = false;
    for (index, &arg) in args.iter().enumerate() {
        if index != 0 && index.is_multiple_of(POLL_STEPS as usize) {
            cx.quittable(Leaf::native_poll)?;
        }
        // The whole string copy precedes the sliced work. The 128 MiB
        // fixture cost 2.14 s/GB for the whole call.
        if let Ok(view) = cx.string_view(arg) {
            // Keep concat's content-based ASCII rule.
            if view.multibyte && !concat_is_ascii(cx, &view.bytes)? {
                multibyte = true;
            }
            parts.push((arg, chunks.len()));
            chunks.push((view.bytes, view.multibyte));
        } else if arg != Value::NIL {
            let values = if cx.pair(arg).is_some() {
                concat_list_chars(cx, arg)?
            } else {
                // The vector clone precedes the encoding polls. The
                // measured whole call cost 1.82 s/GB of vector storage.
                cx.vector_values(arg).map_err(|_| Fault::Wrong("sequencep", arg))?
            };
            let (bytes, multi) = encode_concat_chars(cx, &values)?;
            multibyte |= multi;
            chunks.push((bytes, true));
        }
    }
    assemble_concat(cx, chunks, parts, multibyte)
}

fn concat_list_chars(cx: &mut Leaf<'_>, list: Value) -> std::result::Result<Vec<Value>, Fault> {
    let mut values = Vec::new();
    let mut seen = HashSet::default();
    let mut tail = list;
    while let Some((car, cdr)) = cx.pair(tail) {
        // Concat diagnoses the first repeated cons, with the original list.
        if !seen.insert(tail.0) { return Err(Fault::Circular(list)); }
        if !values.is_empty() && values.len().is_multiple_of(POLL_STEPS as usize) {
            cx.quittable(Leaf::native_poll)?;
        }
        values.push(car);
        tail = cdr;
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", tail)); }
    Ok(values)
}

fn encode_concat_chars(cx: &mut Leaf<'_>, values: &[Value]) -> std::result::Result<(Vec<u8>, bool), Fault> {
    let capacity = if values.len() > POLL_STEPS as usize { values.len() } else { 0 };
    let mut bytes = Vec::with_capacity(capacity);
    let mut multibyte = false;
    for (index, chunk) in values.chunks(POLL_STEPS as usize).enumerate() {
        if index != 0 { cx.quittable(Leaf::native_poll)?; }
        multibyte |= encode_concat_chars_into(&mut bytes, chunk)?;
    }
    Ok((bytes, multibyte))
}

#[inline]
fn encode_concat_chars_into(bytes: &mut Vec<u8>, values: &[Value]) -> std::result::Result<bool, Fault> {
    let mut multibyte = false;
    for &value in values {
        let code = value.integer_value().and_then(|n| u32::try_from(n).ok())
            .filter(|&c| c <= crate::lstring::MAX_CHAR).ok_or_else(|| Fault::Wrong("characterp", value))?;
        multibyte |= (128..=crate::lstring::MAX_5_BYTE_CHAR).contains(&code);
        let mut encoded = [0; 5];
        let n = crate::lstring::char_string(code, &mut encoded);
        bytes.extend_from_slice(&encoded[..n]);
    }
    Ok(multibyte)
}

fn concat_is_ascii(cx: &mut Leaf<'_>, bytes: &[u8]) -> std::result::Result<bool, Fault> {
    for (index, chunk) in bytes.chunks(KERNEL_CHUNK).enumerate() {
        if index != 0 { cx.quittable(Leaf::native_poll)?; }
        if !chunk.is_ascii() { return Ok(false); }
    }
    Ok(true)
}

fn assemble_concat(cx: &mut Leaf<'_>, chunks: Vec<(Vec<u8>, bool)>, mut parts: Vec<(Value, usize)>, multibyte: bool) -> Out {
    let mut bytes = Vec::new();
    let mut offsets = Vec::with_capacity(chunks.len());
    for (index, (chunk, multi)) in chunks.into_iter().enumerate() {
        if index != 0 && index.is_multiple_of(POLL_STEPS as usize) {
            cx.quittable(Leaf::native_poll)?;
        }
        offsets.push(bytes.len());
        if multibyte || !multi || concat_is_ascii(cx, &chunk)? {
            append_concat_bytes(cx, &mut bytes, &chunk, multibyte && !multi)?;
        } else {
            let mut tail = chunk.as_slice();
            let mut steps = crate::quit::Steps::default();
            while !tail.is_empty() {
                if steps.tick() { cx.quittable(Leaf::native_poll)?; }
                let (code, width) = crate::lstring::string_char(tail);
                bytes.push(code as u8);
                tail = &tail[width..];
            }
        }
    }
    for (index, (_, offset)) in parts.iter_mut().enumerate() {
        if index != 0 && index.is_multiple_of(POLL_STEPS as usize) {
            cx.quittable(Leaf::native_poll)?;
        }
        *offset = offsets[*offset];
    }
    Ok(cx.concat_result(bytes, multibyte, &parts))
}

fn append_concat_bytes(cx: &mut Leaf<'_>, bytes: &mut Vec<u8>, chunk: &[u8], widen: bool) -> std::result::Result<(), Fault> {
    for (index, slice) in chunk.chunks(KERNEL_CHUNK).enumerate() {
        if index != 0 { cx.quittable(Leaf::native_poll)?; }
        // In a multibyte result, high unibyte octets become raw-byte characters.
        if widen { bytes.extend(crate::text::octets_to_multibyte(slice)); }
        else { bytes.extend_from_slice(slice); }
    }
    Ok(())
}

defsubr! { name = "indirect-function", gnu = "data.c:2557", effect = Reads,
fn indirect_function(cx, object: Value; optional noerror: Value) {
    // GNU's optional NOERROR is retained for compatibility and ignored.
    // GNU fset prevents cycles; defend corrupt/internal function graphs too.
    let mut current = object;
    let mut seen = Vec::new();
    while current != Value::NIL && cx.is_symbol(current) {
        if seen.contains(&current) { return Err(Fault::CyclicFunctionIndirection(object)); }
        seen.push(current);
        current = cx.symbol_definition(current)?;
    }
    Ok(current)
}}

#[cfg(test)]
mod tests {
    use super::*;

    fn printed(source: &str) -> String {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let value = vm.eval_source_raw(&format!("(prin1-to-string {source})")).expect("primitive evaluator result");
        let Ok(text) = Leaf::new(&mut vm).string_text(value) else { panic!("printer returned non-text") };
        text
    }

    #[test]
    fn widen_symbol_name_runtime() {
        assert_eq!(printed("(symbol-name 'wide-name)"), "\"wide-name\"");
        for source in ["(symbol-name 'wide-name)", "(symbol-name nil)", "(symbol-name t)", "(symbol-name (intern \"\"))"] {
            let mut vm = Vm::new();
            let value = vm.eval_source_raw(source).expect("symbol-name result");
            assert!(vm.str_view(value).is_some_and(|view| !view.is_multibyte()), "{source}: ASCII symbol name must be unibyte");
        }
    }

    #[test]
    fn widen_symbol_name_unicode_remains_multibyte() {
        let mut vm = Vm::new();
        let value = vm.eval_source_raw("(symbol-name 'λ雪😀)").expect("symbol-name result");
        assert!(matches!(vm.object_if(value), Some(data::Object::String(_))));
    }

    #[test]
    fn get_load_suffixes_rejects_gnu_crashing_rep_suffix() {
        assert_eq!(printed("(condition-case e (progn (setq load-file-rep-suffixes (list 42)) (get-load-suffixes)) (error e))"), "(wrong-type-argument sequencep 42)");
    }

    #[test]
    fn get_load_suffixes_empty_does_not_walk_representations() {
        assert_eq!(printed("(condition-case e (let ((load-suffixes nil) (load-file-rep-suffixes (let ((x (list \"\"))) (setcdr x x) x))) (get-load-suffixes)) (error (car e)))"), "nil");
    }

    #[test]
    fn get_load_suffixes_membership_checks_malformed_lists() {
        for (members, expected) in [
            ("7", "(wrong-type-argument listp 7)"),
            ("(cons \"other\" 7)", "(wrong-type-argument listp (\"other\" . 7))"),
        ] {
            assert_eq!(printed(&format!("(condition-case e (progn (setq jka-compr-load-suffixes {members} load-suffixes (list \".so\") load-file-rep-suffixes (list \".gz\")) (get-load-suffixes)) (error e))")), expected);
        }
    }

    #[test]
    fn get_load_suffixes_membership_short_circuits() {
        for members in ["(cons \".gz\" 7)", "(let ((x (list \".gz\"))) (setcdr x x) x)"] {
            assert_eq!(printed(&format!("(condition-case e (progn (setq jka-compr-load-suffixes {members} load-suffixes (list \".so\") load-file-rep-suffixes (list \".gz\")) (get-load-suffixes)) (error (car e)))")), "nil");
        }
    }

    #[test]
    fn widen_concat_runtime() {
        assert_eq!(printed("(concat \"a\" \"λ\" \"b\")"), "\"aλb\"");
    }

    #[test]
    fn widen_concat_long_list_and_cycle() {
        let mut vm = Vm::new();
        let mut cx = Leaf::new(&mut vm);
        let mut list = Value::NIL;
        for _ in 0..10_000 { list = cx.cons(Value::integer(65).unwrap(), list); }
        let Body::Leaf(body) = concat::DESC.body else { panic!("expected leaf") };
        let result = body(&mut cx, &[list]).unwrap_or_else(|_| panic!("proper list concat"));
        assert_eq!(cx.string_bytes(result).unwrap_or_else(|_| panic!("string result")), vec![65; 10_000]);
        let cycle = cx.cons(Value::integer(65).unwrap(), Value::NIL);
        cx.setcdr(cycle, cycle).unwrap_or_else(|_| panic!("setcdr"));
        assert!(matches!(body(&mut cx, &[cycle]), Err(Fault::Circular(v)) if v == cycle));
    }

    #[test]
    fn widen_optional_many_and_exact_faults() {
        assert_eq!((symbol_name::DESC.min, symbol_name::DESC.max), (1, Some(1)));
        assert_eq!((concat::DESC.min, concat::DESC.max), (0, None));
        assert_eq!((indirect_function::DESC.min, indirect_function::DESC.max), (1, Some(2)));
        for (source, expected) in [
            ("(concat)", "\"\""),
            ("(concat nil '(65 66) [955] \"雪\")", "\"ABλ雪\""),
            ("(indirect-function 'wide-unbound)", "nil"),
            ("(indirect-function 'wide-unbound 'anything)", "nil"),
            ("(indirect-function '(lambda (x) x))", "(lambda (x) x)"),
            ("(condition-case e (symbol-name 7) (error e))", "(wrong-type-argument symbolp 7)"),
            ("(condition-case e (concat '(bad . tail)) (error e))", "(wrong-type-argument listp tail)"),
            ("(condition-case e (concat [65 bad]) (error e))", "(wrong-type-argument characterp bad)"),
        ] { assert_eq!(printed(source), expected, "{source}"); }
    }

    #[test]
    fn widen_empty_intern_runtime() {
        assert_eq!(printed("(symbol-name (intern \"\"))"), "\"\"");
    }

    #[test]
    fn widen_fset_lambda_runtime() {
        assert_eq!(printed("(progn (fset 'wide-b '(lambda (x) x)) (fset 'wide-a 'wide-b) (indirect-function 'wide-a))"), "(lambda (x) x)");
    }

    #[test]
    fn widen_fset_cycle_blame_runtime() {
        assert_eq!(printed("(condition-case e (progn (fset 'wide-a 'wide-b) (fset 'wide-b 'wide-a)) (error e))"), "(cyclic-function-indirection wide-b)");
    }

    #[test]
    fn widen_indirect_function_runtime() {
        assert_eq!(printed("(indirect-function 'wide-unbound t)"), "nil");
    }
}

defsubr! { name = "get-load-suffixes", gnu = "lread.c:961", effect = Reads,
fn get_load_suffixes(cx) {
    let load = cx.intern("load-suffixes");
    let reps = cx.intern("load-file-rep-suffixes");
    let suffixes = cx.variable_value(load)?.ok_or_else(|| Fault::VoidVariable(load))?;
    let exts = cx.variable_value(reps)?.ok_or_else(|| Fault::VoidVariable(reps))?;
    let mut out = Vec::new();
    let mut suffix_tail = suffixes;
    let mut seen_suffixes = HashSet::default();
    while let Some((suffix, next_suffix)) = cx.pair(suffix_tail) {
        if !seen_suffixes.insert(suffix_tail.0) { return Err(Fault::Circular(suffixes)); }
        let mut ext_tail = exts;
        let mut seen_exts = HashSet::default();
        while let Some((ext, next_ext)) = cx.pair(ext_tail) {
            if !seen_exts.insert(ext_tail.0) { return Err(Fault::Circular(exts)); }
            ext_tail = next_ext;
            if cx.string_bytes(ext).is_ok_and(|b| !b.is_empty())
                && cx.string_bytes(suffix).is_ok_and(|b| b.ends_with(b".so"))
            {
                let jka = cx.intern("jka-compr-load-suffixes");
                let list = cx.variable_value(jka)?.ok_or_else(|| Fault::VoidVariable(jka))?;
                if suffix_member(cx, ext, list)? { continue; }
            }
            out.push(concat(cx, &[suffix, ext])?);
        }
        suffix_tail = next_suffix;
    }
    Ok(out.iter().rev().fold(Value::NIL, |tail, &v| cx.cons(v, tail)))
}}

defsubr! { name = "set-match-data", gnu = "search.c:2949", effect = Global,
fn set_match_data(cx, list: Value; optional reseat: Value) {
    cx.set_match_data(list, reseat)
}}

fn suffix_member(cx: &Leaf<'_>, needle: Value, list: Value) -> std::result::Result<bool, Fault> {
    let mut seen = HashSet::default();
    let mut tail = list;
    while let Some((item, next)) = cx.pair(tail) {
        if !seen.insert(tail.0) { return Err(Fault::Circular(list)); }
        let same = cx.equal(needle, item).map_err(|e| match e {
            crate::hash::HashFault::Wrong(p, v) => Fault::Wrong(p, v),
            crate::hash::HashFault::Error("circular-list", Some(v)) => Fault::Circular(v),
            crate::hash::HashFault::Error(m, _) => Fault::Error(m.into()),
            _ => Fault::Error("equal failed".into()),
        })?;
        if same { return Ok(true); }
        tail = next;
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", list)); }
    Ok(false)
}

// `concat' of the elements of a list with SEPARATOR between each two (nil
// for none): Fmapconcat's Fconcat over its results, which the prelude's
// `mapconcat' collects (lisp/vm/prelude.el). The parts are walked
// once; ASCII strings are joined without building the interleaved list.
defsubr! { name = "romacs--concat-list", gnu = "fns.c:3484", effect = Pure,
fn concat_list(cx, parts: Value; optional separator: Value) {
    let parts = list_elements(cx, parts)?;
    let separator = (separator != Value::NIL && cx.string_chars(separator) != Some(0)).then_some(separator);
    if let Some(result) = cx.concat_ascii_joined::<true>(&parts, separator)? { return Ok(result); }
    let Some(separator) = separator else { return concat(cx, &parts) };
    let mut joined = Vec::with_capacity(2 * parts.len());
    for (index, &part) in parts.iter().enumerate() {
        if index != 0 && index.is_multiple_of(POLL_STEPS as usize) {
            cx.quittable(Leaf::native_poll)?;
        }
        if index > 0 { joined.push(separator); }
        joined.push(part);
    }
    concat(cx, &joined)
}}

/// The elements of LIST, a proper list a mapping function built.
pub(super) fn list_elements(cx: &mut Leaf<'_>, list: Value) -> std::result::Result<Vec<Value>, Fault> {
    let mut values = Vec::new();
    let (mut tail, mut tortoise, mut steps) = (list, list, 0u64);
    while let Some((car, rest)) = cx.pair(tail) {
        values.push(car);
        tail = rest;
        if tail == tortoise { return Err(Fault::Circular(tail)); }
        steps += 1;
        if steps & 4095 == 0 {
            if steps.is_power_of_two() { tortoise = tail; }
            cx.quittable(Leaf::native_poll)?;
        }
    }
    if tail != Value::NIL {
        return Err(Fault::Wrong("listp", tail));
    }
    Ok(values)
}
