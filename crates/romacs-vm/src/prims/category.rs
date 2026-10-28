//! GNU category.c primitives over the leaf category-table substrate.
use super::*;
use crate::prim::leaf::Kind;

/// CHECK_CATEGORY: a character from ` ' to `~'.
fn category(cx: &Leaf<'_>, value: Value) -> std::result::Result<usize, Fault> {
    let _ = cx;
    value.integer_value().filter(|c| (32..=126).contains(c)).map(|c| c as usize).ok_or(Fault::Wrong("categoryp", value))
}

fn character(value: Value) -> std::result::Result<usize, Fault> {
    value.integer_value().filter(|c| (0..=0x3fffff).contains(c)).map(|c| c as usize).ok_or(Fault::Wrong("characterp", value))
}

defsubr! { name = "make-category-set", gnu = "category.c:72", effect = Pure,
fn make_category_set(cx, categories: Value) {
    if !matches!(cx.kind(categories), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", categories)); }
    let view = cx.string_view(categories)?;
    if view.is_multibyte() && !view.bytes().is_ascii() {
        return Err(Fault::Error("Multibyte string in \u{2018}make-category-set\u{2019}".into()));
    }
    let mut bits = crate::data::BoolVector::new(128, false).map_err(Fault::Error)?;
    for &byte in view.bytes().iter().rev() {
        let c = category(cx, Value::integer(i64::from(byte)).unwrap())?;
        bits.set(c, true);
    }
    Ok(cx.bool_vector_from(bits))
}}

defsubr! { name = "define-category", gnu = "category.c:105", effect = Global,
fn define_category(cx, cat: Value, docstring: Value; optional table: Value) {
    let c = category(cx, cat)?;
    if !matches!(cx.kind(docstring), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", docstring)); }
    let table = cx.check_category_table(table)?;
    if cx.category_docstring(table, c)? != Value::NIL {
        return Err(Fault::Error(format!("Category \u{2018}{}\u{2019} is already defined", c as u8 as char)));
    }
    cx.set_category_docstring(table, c, docstring)?;
    Ok(Value::NIL)
}}

defsubr! { name = "category-docstring", gnu = "category.c:126", effect = Reads,
fn category_docstring(cx, cat: Value; optional table: Value) {
    let c = category(cx, cat)?;
    let table = cx.check_category_table(table)?;
    cx.category_docstring(table, c)
}}

defsubr! { name = "get-unused-category", gnu = "category.c:138", effect = Reads,
fn get_unused_category(cx; optional table: Value) {
    let table = cx.check_category_table(table)?;
    for c in 32..=126 {
        if cx.category_docstring(table, c)? == Value::NIL { return Ok(Value::integer(c as i64).unwrap()); }
    }
    Ok(Value::NIL)
}}

defsubr! { name = "category-table-p", gnu = "category.c:160", effect = Pure,
fn category_table_p(cx, arg: Value) { Ok(if cx.is_category_table(arg) { Value::TRUE } else { Value::NIL }) }}

defsubr! { name = "category-table", gnu = "category.c:184", effect = Reads,
fn category_table(cx) { cx.current_category_table() }}

defsubr! { name = "standard-category-table", gnu = "category.c:192", effect = Reads,
fn standard_category_table(cx) { Ok(cx.standard_category_table()) }}

defsubr! { name = "copy-category-table", gnu = "category.c:232", effect = Global,
fn copy_category_table(cx; optional table: Value) {
    let table = if table == Value::NIL { cx.standard_category_table() } else { cx.check_category_table(table)? };
    cx.copy_category_table(table)
}}

defsubr! { name = "make-category-table", gnu = "category.c:246", effect = Global,
fn make_category_table(cx) { Ok(cx.make_category_table()) }}

defsubr! { name = "set-category-table", gnu = "category.c:262", effect = Global,
fn set_category_table(cx, table: Value) {
    let table = cx.check_category_table(table)?;
    cx.set_current_category_table(table)?;
    Ok(table)
}}

defsubr! { name = "char-category-set", gnu = "category.c:283", effect = Reads,
fn char_category_set(cx, ch: Value) {
    let c = character(ch)?;
    let table = cx.current_category_table()?;
    cx.char_table_element(table, c)
}}

defsubr! { name = "category-set-mnemonics", gnu = "category.c:292", effect = Pure,
fn category_set_mnemonics(cx, set: Value) {
    let bits = cx.bool_vector(set).ok().filter(|b| b.len == 128).ok_or(Fault::Wrong("categorysetp", set))?;
    let text: String = (32..127).filter(|&i| bits.get(i) == Some(true)).map(|i| i as u8 as char).collect();
    Ok(cx.string(&text))
}}

defsubr! { name = "modify-category-entry", gnu = "category.c:314", effect = Global,
fn modify_category_entry(cx, ch: Value, cat: Value; optional table: Value, reset: Value) {
    let (start, end) = if ch.integer_value().is_some() { let c = character(ch)?; (c, c) } else {
        let (from, to) = cx.pair(ch).ok_or(Fault::Wrong("consp", ch))?;
        (character(from)?, character(to)?)
    };
    let c = category(cx, cat)?;
    let table = cx.check_category_table(table)?;
    if cx.category_docstring(table, c)? == Value::NIL {
        return Err(Fault::Error(format!("Undefined category: {}", c as u8 as char)));
    }
    cx.modify_category_entry(table, start, end, c, reset == Value::NIL)?;
    Ok(Value::NIL)
}}
