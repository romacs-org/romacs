//! GNU syntax.c character-set motion, without syntax-table interpretation.
use super::*;

defsubr! { name = "skip-chars-forward", gnu = "syntax.c:1618", effect = Global,
fn skip_chars_forward(cx, string: Value; optional limit: Value) {
    cx.skip_characters(string, limit, true)
}}

defsubr! { name = "skip-chars-backward", gnu = "syntax.c:1633", effect = Global,
fn skip_chars_backward(cx, string: Value; optional limit: Value) {
    cx.skip_characters(string, limit, false)
}}

defsubr! { name = "skip-syntax-forward", gnu = "syntax.c:1642", effect = Global,
fn skip_syntax_forward(cx, syntax: Value; optional limit: Value) {
    cx.skip_syntax(syntax, limit, true)
}}

defsubr! { name = "skip-syntax-backward", gnu = "syntax.c:1653", effect = Global,
fn skip_syntax_backward(cx, syntax: Value; optional limit: Value) {
    cx.skip_syntax(syntax, limit, false)
}}

defsubr! { name = "backward-prefix-chars", gnu = "syntax.c:3096", effect = Global,
fn backward_prefix_chars(cx) { cx.backward_prefix_chars() }}
