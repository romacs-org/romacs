//! Native syntax-table word-motion prerequisite for GNU Lisp commands.
use super::*;

defsubr! { name = "forward-word", gnu = "syntax.c:1581", effect = Global,
fn forward_word(cx; optional count: Value) {
    cx.move_words(count)
}}
