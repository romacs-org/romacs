//! GNU char-table construction and extra slots.
use super::*;

defsubr! { name = "make-char-table", gnu = "chartab.c:104", effect = Reads,
fn make_char_table(cx, purpose: Value; optional init: Value) {
    cx.make_char_table(purpose, init)
}}

defsubr! { name = "char-table-extra-slot", gnu = "chartab.c:548", effect = Reads,
fn char_table_extra_slot(cx, table: Value, n: Value) {
    cx.char_table_extra(table, n, None)
}}

defsubr! { name = "set-char-table-extra-slot", gnu = "chartab.c:562", effect = MutatesArgs,
fn set_char_table_extra_slot(cx, table: Value, n: Value, value: Value) {
    cx.char_table_extra(table, n, Some(value))
}}

defsubr! { name = "char-table-parent", gnu = "chartab.c:510", effect = Reads,
fn char_table_parent(cx, table: Value) {
    cx.char_table_parent(table, None)
}}

defsubr! { name = "set-char-table-parent", gnu = "chartab.c:524", effect = MutatesArgs,
fn set_char_table_parent(cx, table: Value, parent: Value) {
    cx.char_table_parent(table, Some(parent))
}}

defsubr! { name = "set-char-table-range", gnu = "chartab.c:609", effect = MutatesArgs,
fn set_char_table_range(cx, table: Value, range: Value, value: Value) {
    cx.set_char_table_range(table, range, value)
}}


// Unicode property (uniprop) tables, GNU chartab.c's lower half.

fn uniprop_args(cx: &Leaf<'_>, table: Value, ch: Value) -> std::result::Result<usize, Fault> {
    if cx.kind(table) != crate::prim::leaf::Kind::CharTable { return Err(Fault::Wrong("char-table-p", table)); }
    let c = ch.integer_value().filter(|&c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c))
        .ok_or(Fault::Wrong("characterp", ch))? as usize;
    if !cx.is_uniprop(table) { return Err(Fault::Error("Invalid Unicode property table".into())); }
    Ok(c)
}

fn extra(cx: &mut Leaf<'_>, table: Value, n: i64) -> Out {
    cx.char_table_extra(table, Value::integer(n).unwrap(), None)
}

// GNU Fassq over `char-code-property-alist'.
fn property_entry(cx: &mut Leaf<'_>, prop: Value) -> Out {
    let symbol = cx.intern("char-code-property-alist");
    let alist = cx.variable_value(symbol)?.unwrap_or(Value::NIL);
    let mut tail = alist;
    for _ in 0..=cx.list_limit() {
        let Some((element, rest)) = cx.pair(tail) else {
            return if tail == Value::NIL { Ok(Value::NIL) } else { Err(Fault::Wrong("listp", alist)) };
        };
        if cx.pair(element).is_some_and(|(key, _)| key == prop) { return Ok(element); }
        tail = rest;
    }
    Err(Fault::Circular(alist))
}

// uniprop_table loads a file-name entry, whose define-char-code-property
// replaces it. Either way the result is the entry's current cdr, the
// table or, when the file did not provide one, the file name. A table
// whose decoder C knows gets its ASCII cache re-derived (the printed
// table carries a separate copy of that block), as GNU does. VM2's
// `load' ignores coding-system-for-read, so GNU's utf-8-emacs-unix binding
// around it has nothing to protect.
defsubr! { name = "unicode-property-table-internal", gnu = "chartab.c:1303", effect = Global,
fn unicode_property_table_internal(cx, prop: Value) -> Step {
    let entry = property_entry(cx, prop)?;
    let file = cx.pair(entry).map_or(Value::NIL, |(_, table)| table);
    if !matches!(cx.kind(file), crate::prim::leaf::Kind::String | crate::prim::leaf::Kind::Bytes) {
        return Ok(Step::Done(prepared(cx, file)?));
    }
    let path = format!("international/{}", cx.string_text(file)?);
    let path = cx.string(&path);
    Ok(Step::Call { f: cx.intern("load"), args: vec![path, Value::TRUE, Value::TRUE, Value::TRUE, Value::TRUE],
        then: property_loaded, saved: vec![prop] })
}}
fn property_loaded(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let entry = property_entry(cx, saved[0])?;
    let table = cx.pair(entry).map_or(Value::NIL, |(_, table)| table);
    Ok(Step::Done(prepared(cx, table)?))
}
fn prepared(cx: &mut Leaf<'_>, table: Value) -> Out {
    if cx.kind(table) == crate::prim::leaf::Kind::CharTable && cx.is_uniprop(table)
        && extra(cx, table, 1).map(|d| d == Value::NIL || d.integer_value() == Some(0))? {
        cx.uniprop_reset_ascii(table);
    }
    Ok(table)
}

defsubr! { name = "get-unicode-property-internal", gnu = "chartab.c:1326", effect = Reads,
fn get_unicode_property_internal(cx, table: Value, ch: Value) {
    let c = uniprop_args(cx, table, ch)?;
    let value = cx.char_table_element(table, c)?;
    if cx.map_table_has_decoder(table) { cx.map_table_decode(table, value) } else { Ok(value) }
}}

defsubr! { name = "put-unicode-property-internal", gnu = "chartab.c:1339", effect = MutatesArgs,
fn put_unicode_property_internal(cx, table: Value, ch: Value, value: Value) {
    let c = uniprop_args(cx, table, ch)?;
    let encoded = match extra(cx, table, 2)?.integer_value() {
        // uniprop_encode_value_character
        Some(0) => {
            if value != Value::NIL && !value.integer_value().is_some_and(|v| (0..=crate::lstring::MAX_CHAR as i64).contains(&v)) {
                return Err(Fault::Wrong("integerp", value));
            }
            value
        }
        // uniprop_encode_value_run_length and _numeric: VALUE's index in the
        // value vector (extra slot 4).
        Some(kind @ (1 | 2)) => {
            if kind == 2 && value.integer_value().is_none() { return Err(Fault::Wrong("fixnump", value)); }
            let values = extra(cx, table, 4)?;
            let mut known = cx.vector_values(values)?;
            let index = known.iter().position(|&v| v == value).unwrap_or(known.len());
            let encoded = Value::integer(index as i64).unwrap();
            if index == known.len() {
                if kind == 1 {
                    let text = cx.string("Unicode property value");
                    let data = cx.cons(value, Value::NIL);
                    let data = cx.cons(text, data);
                    return Err(Fault::Signal(cx.intern("wrong-type-argument"), data));
                }
                // GNU appends the index it returns, not VALUE (chartab.c:1231).
                known.push(encoded);
                let grown = cx.vector_from(known);
                cx.char_table_extra(table, Value::integer(4).unwrap(), Some(grown))?;
            }
            encoded
        }
        _ => value,
    };
    cx.uniprop_expand(table, c)?;
    cx.set_char_table_range(table, Value::integer(c as i64).unwrap(), encoded)?;
    Ok(Value::NIL)
}}
