//! Widened declaration proving primitives and evaluator regression witnesses.
use super::*;

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
    let key = cx.string_view(name).map_err(|_| Fault::Wrong("stringp", name)).map(|v| crate::obarray::symbol_key(v.bytes(), v.len()))?;
    Ok(cx.make_symbol(&key.ok_or(Fault::NotUnicode(name))?))
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
    if let Some(result)=cx.concat_ascii(args) { return Ok(result); }
    let mut chunks: Vec<(Vec<u8>, bool)> = Vec::new();
    let mut parts = Vec::new();
    let mut multibyte = false;
    let mut raw_byte_arg = None;
    for &arg in args {
        if let Ok(view) = cx.string_view(arg) {
            // P1 preserves concat's old content-based ASCII rule. P2 owns kind
            // propagation and widening unibyte high octets in mixed results.
            if view.multibyte {
                multibyte |= !view.bytes.is_ascii();
            } else if !view.bytes.is_ascii() { raw_byte_arg = Some(arg); }
            parts.push((arg, chunks.len()));
            chunks.push((view.bytes, view.multibyte));
            continue;
        }
        let values = if arg == Value::NIL { continue; }
        else if cx.pair(arg).is_some() {
            let mut values = Vec::new(); let mut seen = HashSet::default(); let mut tail = arg;
            while let Some((car,cdr))=cx.pair(tail) {
                if !seen.insert(tail.0) { return Err(Fault::Circular(arg)); }
                values.push(car); tail=cdr;
            }
            if tail != Value::NIL { return Err(Fault::Wrong("listp",tail)); }
            values
        } else { cx.vector_values(arg).map_err(|_|Fault::Wrong("sequencep",arg))? };
        let mut bytes=Vec::new();
        for value in values {
            let code=value.integer_value().and_then(|n|u32::try_from(n).ok()).filter(|&c|c<=crate::lstring::MAX_CHAR).ok_or(Fault::Wrong("characterp",value))?;
            multibyte |= code >= 128 && code <= crate::lstring::MAX_5_BYTE_CHAR;
            let mut encoded=[0;5]; let n=crate::lstring::char_string(code,&mut encoded); bytes.extend_from_slice(&encoded[..n]);
        }
        chunks.push((bytes,true));
    }
    // A unibyte part of a multibyte result: its bytes above 127 become
    // raw-byte characters (fns.c concat, str_to_multibyte).
    let _ = raw_byte_arg;
    let mut bytes=Vec::new();
    let mut offsets=Vec::with_capacity(chunks.len());
    for (chunk,multi) in chunks {
        offsets.push(bytes.len());
        if multibyte && !multi { bytes.extend(crate::text::octets_to_multibyte(&chunk)); }
        else if multibyte || !multi || chunk.is_ascii() { bytes.extend_from_slice(&chunk); }
        else {
            let mut tail=chunk.as_slice();
            while !tail.is_empty() { let (code,n)=crate::lstring::string_char(tail); bytes.push(code as u8); tail=&tail[n..]; }
        }
    }
    let parts=parts.into_iter().map(|(value,chunk)| (value,offsets[chunk])).collect::<Vec<_>>();
    Ok(cx.concat_result(bytes,multibyte,&parts))
}}

defsubr! { name = "indirect-function", gnu = "data.c:2557", effect = Reads,
fn indirect_function(cx, object: Value; optional noerror: Value) {
    // GNU's optional NOERROR is retained for compatibility and ignored.
    // GNU fset prevents cycles; defend corrupt/internal function graphs too.
    let mut current = object;
    let mut seen = Vec::new();
    while current != Value::NIL && cx.symbol_name(current).is_ok() {
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
    let suffixes = cx.variable_value(load)?.ok_or(Fault::VoidVariable(load))?;
    let exts = cx.variable_value(reps)?.ok_or(Fault::VoidVariable(reps))?;
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
                let list = cx.variable_value(jka)?.ok_or(Fault::VoidVariable(jka))?;
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

// `concat' of the elements of a list: Fmapconcat's Fconcat over its
// results, which the prelude's `mapconcat' collects (rlisp/vm/prelude.el).
defsubr! { name = "romacs--concat-list", gnu = "fns.c:3484", effect = Pure,
fn concat_list(cx, parts: Value) {
    let parts = list_elements(cx, parts)?;
    concat(cx, &parts)
}}

/// The elements of LIST, a proper list a mapping function built.
pub(super) fn list_elements(cx: &Leaf<'_>, list: Value) -> std::result::Result<Vec<Value>, Fault> {
    let mut values = Vec::new();
    let (mut tail, mut tortoise, mut steps) = (list, list, 0);
    while let Some((car, _)) = cx.pair(tail) {
        values.push(car);
        (tail, tortoise, steps) = super::fns_equal::for_each_tail_step(cx, tail, tortoise, steps)?;
    }
    if tail != Value::NIL {
        return Err(Fault::Wrong("listp", tail));
    }
    Ok(values)
}
