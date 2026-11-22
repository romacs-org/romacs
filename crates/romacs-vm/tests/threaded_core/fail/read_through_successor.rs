// An operand read through the successor, which may be the terminator.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Bytes, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Bytes>) -> Value {
        let next = pc.next();
        r.window(&mut e)[pc.a()] = next.value();
        go(e, r, next)
    }
}
