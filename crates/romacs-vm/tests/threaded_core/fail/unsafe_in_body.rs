// `unsafe` in a handler's body.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Bytes, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(e: Hot<'f, '_, '_>, r: Regs<'f>, pc: Pc<'f, Bytes>) -> Value {
        // SAFETY: none; the misuse is that the block exists at all.
        let value = unsafe { std::ptr::read(&pc.value()) };
        let _ = value;
        go(e, r, pc.next())
    }
}
