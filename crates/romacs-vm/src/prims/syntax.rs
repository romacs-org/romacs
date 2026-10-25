use super::*;

defsubr! { name = "string-to-syntax", gnu = "syntax.c:1156", effect = Reads,
fn string_to_syntax(cx, string: Value) { cx.string_to_syntax(string) }}
