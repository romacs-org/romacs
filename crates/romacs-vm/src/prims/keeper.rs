//! The keeper's one leaf primitive: what `rlisp/romacs-keeper.el` is built
//! on. Not in GNU (ledger/divergences/VM2-KEEPER.md).
use super::Value;

// (romacs--keeper-call OP &rest ARGS): ask this session's keeper, or act
// on the descriptors it handed over. OP is a symbol; romacs-keeper.el has
// one documented function per OP.
defsubr! { name = "romacs--keeper-call", gnu = "romacs:keeper", effect = Global,
fn keeper_call(cx, op: Value; rest rest) {
    cx.keeper_call(op, rest)
}}
