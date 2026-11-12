// `unsafe` in a handler's body.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Pc, Regs};
    use crate::{Eng, Value};

    pub(super) fn body(e: Eng<'_, '_>, r: Regs, pc: Pc) -> Value {
        // SAFETY: none; the misuse is that the block exists at all.
        let value = unsafe { std::ptr::read(&pc.value()) };
        let _ = value;
        go(e, r, pc.next())
    }
}
