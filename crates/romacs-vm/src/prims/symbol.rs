//! Symbol property lists from GNU `src/data.c`.
//!
//! A plist belongs to the symbol OBJECT (`struct Lisp_Symbol` `u.s.plist`,
//! lisp.h), so an uninterned symbol from `make-symbol` carries its own and
//! two symbols sharing a name never share one. The plist is a moving heap
//! Value held by the symbol's slot; the collector marks and forwards it with
//! the slot (gc.rs), never through a name.
use super::*;
use crate::prim::leaf::Kind;

// GNU tests the distinct PVEC_SYMBOL_WITH_POS kind, not SYMBOLP and not a
// property of the bare symbol. VM2 has no such heap kind or producer (neither
// position-symbol nor read-positioning-symbols); its reader interns bare
// symbols. Thus the predicate is false throughout the representable domain.
// See VM2-SYMPOS1 and tests/symbol_with_pos.rs: adding a producer requires
// replacing this implementation, with traced symbol/position storage.
defsubr! { name = "symbol-with-pos-p", gnu = "data.c:362", effect = Pure,
fn symbol_with_pos_p(_cx, _object: Value) {
    Ok(Value::NIL)
}}

defsubr! { name = "symbol-plist", gnu = "data.c:818", effect = Reads,
fn symbol_plist(cx, symbol: Value) {
    cx.symbol_plist(symbol)
}}

defsubr! { name = "setplist", gnu = "data.c:1029", effect = Global,
fn setplist(cx, symbol: Value, newplist: Value) {
    cx.set_symbol_plist(symbol, newplist)
}}

// GNU SUBRP tests PVEC_SUBR, including native-compiled Lisp. VM2 compiled
// Lisp closures retain Kind::Closure; they are not native-comp subrs.
defsubr! { name = "subrp", gnu = "data.c:510", effect = Pure,
fn subrp(cx, object: Value) {
    Ok(if cx.kind(object) == Kind::Subr { Value::TRUE } else { Value::NIL })
}}

// GNU requires a closure whose CLOSURE_CODE is a string (data.c:536).
// VM2 closures contain its own register-code representation, not GNU bytecode;
// neither the GNU #[...] reader nor make-byte-code is implemented. Thus no
// representable object satisfies this predicate. See VM2-BYTECODE1; adding a
// GNU bytecode producer requires replacing this negative-only implementation.
defsubr! { name = "byte-code-function-p", gnu = "data.c:531", effect = Pure,
fn byte_code_function_p(_cx, _object: Value) {
    Ok(Value::NIL)
}}
