//! GNU editfns.c:3422: requote template literals, not substituted arguments.
use super::*;

defsubr! { name = "format-message", gnu = "editfns.c:3422", effect = Reads,
fn format_message(cx, template: Value; rest arguments) {
    cx.format_message(template, arguments)
}}
