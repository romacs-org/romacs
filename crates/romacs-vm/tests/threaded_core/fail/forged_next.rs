// A successor made outside the core.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Next, Pc, Regs};
    use crate::{Eng, Value};

    pub(super) fn body(e: Eng<'_, '_>, r: Regs, pc: Pc) -> Value {
        let _ = pc;
        go(e, r, Next(std::ptr::null()))
    }
}
