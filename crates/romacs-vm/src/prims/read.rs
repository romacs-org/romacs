//! GNU reader entry point; delegates to the VM reader, not an evaluator shim.
use super::*;

defsubr! { name = "read-from-string", gnu = "lread.c:2511", effect = Global,
fn read_from_string(cx, string: Value; optional start: Value, end: Value) -> Reader {
    cx.read_from_string(string, start, end)
}}

defsubr! { name = "read", gnu = "lread.c:2457", effect = Global,
fn read(cx; optional stream: Value) -> Reader {
    cx.read(stream)
}}
