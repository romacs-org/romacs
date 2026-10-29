use super::*;

defsubr! { name = "string-to-syntax", gnu = "syntax.c:1156", effect = Reads,
fn string_to_syntax(cx, string: Value) { cx.string_to_syntax(string) }}

defsubr! { name = "syntax-table-p", gnu = "syntax.c:984", effect = Pure,
fn syntax_table_p(cx, object: Value) {
    use crate::prim::leaf::Kind;
    let table = matches!(cx.kind(object), Kind::CharTable | Kind::SyntaxTable)
        && cx.char_table_subtype(object)? == cx.intern("syntax-table");
    Ok(if table { Value::TRUE } else { Value::NIL })
}}
