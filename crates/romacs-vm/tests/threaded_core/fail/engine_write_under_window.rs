// A mutable use of the engine while a window lives.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f>) -> Value {
        let regs = r.window(&mut e);
        let spent = e.spend();
        regs[pc.a()] = if spent { Value::NIL } else { Value::TRUE };
        go(e, r, pc.next())
    }
}
