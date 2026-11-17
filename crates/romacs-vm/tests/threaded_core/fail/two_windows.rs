// Two windows on one frame at once.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f>) -> Value {
        let regs = r.window(&mut e);
        let other = r.window(&mut e);
        regs[pc.a()] = other[pc.b()];
        go(e, r, pc.next())
    }
}
