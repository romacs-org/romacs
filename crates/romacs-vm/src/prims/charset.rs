use super::*;

defsubr! { name = "charset-plist", gnu = "charset.c:1295", effect = Reads,
fn charset_plist(cx, charset: Value) { cx.charset_plist(charset) }}

defsubr! { name = "set-charset-plist", gnu = "charset.c:1306", effect = Global,
fn set_charset_plist(cx, charset: Value, plist: Value) { cx.set_charset_plist(charset, plist) }}

defsubr! { name = "define-charset-alias", gnu = "charset.c:1281", effect = Global,
fn define_charset_alias(cx, alias: Value, charset: Value) { cx.charset_alias(alias, charset) }}

