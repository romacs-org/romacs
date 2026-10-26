use super::*;

defsubr! { name = "charset-plist", gnu = "charset.c:1295", effect = Reads,
fn charset_plist(cx, charset: Value) { cx.charset_plist(charset) }}

defsubr! { name = "set-charset-plist", gnu = "charset.c:1306", effect = Global,
fn set_charset_plist(cx, charset: Value, plist: Value) { cx.set_charset_plist(charset, plist) }}

defsubr! { name = "define-charset-alias", gnu = "charset.c:1281", effect = Global,
fn define_charset_alias(cx, alias: Value, charset: Value) { cx.charset_alias(alias, charset) }}


defsubr! { name = "unify-charset", gnu = "charset.c:1318", effect = Global,
fn unify_charset(cx, charset: Value; optional unify_map: Value, deunify: Value) { cx.unify_charset(charset, unify_map, deunify) }}

defsubr! { name = "charsetp", gnu = "charset.c:642", effect = Reads,
fn charsetp(cx, object: Value) { Ok(if cx.is_charset(object) { Value::TRUE } else { Value::NIL }) }}

defsubr! { name = "set-charset-priority", gnu = "charset.c:2165", effect = Global,
fn set_charset_priority(cx, first: Value; rest others) {
    let mut charsets = vec![first];
    charsets.extend_from_slice(others);
    cx.set_charset_priority(&charsets)
}}

defsubr! { name = "charset-priority-list", gnu = "charset.c:2146", effect = Reads,
fn charset_priority_list(cx; optional highestp: Value) { cx.charset_priority_list(highestp) }}
