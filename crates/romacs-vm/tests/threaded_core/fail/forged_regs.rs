// A register capability made outside the core.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Bytes, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(e: Hot<'f, '_, '_>, r: Regs<'f>, pc: Pc<'f, Bytes>) -> Value {
        drop(r);
        go(e, Regs(std::ptr::null_mut()), pc.next())
    }
}
