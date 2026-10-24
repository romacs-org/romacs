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
