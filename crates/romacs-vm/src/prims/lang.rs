//! Core language predicates.
//!
//! GNU implements these in `src/data.c` as `attributes: const` DEFUNs: total
//! functions of one argument that signal nothing. They are `Effect::Pure` here
//! for the same reason, which makes them stageable.
use super::*;

// GNU data.c:321 returns nil for a cons and t for everything else, so nil,
// vectors, hash tables and all numbers are atoms. The complement of consp,
// exactly; `tests/lang_atom.rs` asserts that complement over GNU's own answers
// so the two can never drift apart.
defsubr! { name = "atom", gnu = "data.c:321", effect = Pure,
fn atom(cx, object: Value) {
    Ok(if cx.pair(object).is_some() { Value::NIL } else { Value::TRUE })
}}

// GNU data.c:541: t iff OBJECT is a closure whose code slot is a cons, i.e.
// an interpreted function. VM2 loads no .elc and has no byte-code objects, so
// every closure it builds from Lisp source -- eagerly or lazily compiled
// (R215) -- is one GNU would hold interpreted; a primitive is a subr. GNU
// itself answers t for eagerly macroexpanded closures too, so this does NOT
// report whether a body is still unexpanded. loadup.el:181 branches on it.
defsubr! { name = "interpreted-function-p", gnu = "data.c:541", effect = Pure,
fn interpreted_function_p(cx, object: Value) {
    Ok(if cx.kind(object) == crate::prim::leaf::Kind::Closure { Value::TRUE } else { Value::NIL })
}}
