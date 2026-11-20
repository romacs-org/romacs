// A successor made outside the core.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Bytes, Hot, Next, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(e: Hot<'f, '_, '_>, r: Regs<'f>, pc: Pc<'f, Bytes>) -> Value {
        let _ = pc;
        go(e, r, Next(std::ptr::null()))
    }
}
