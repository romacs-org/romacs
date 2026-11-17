// A shared read of the engine while a window lives.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f>) -> Value {
        let regs = r.window(&mut e);
        let pair = e.pair(0);
        regs[pc.a()] = pair.map_or(Value::NIL, |p| p.0);
        go(e, r, pc.next())
    }
}
