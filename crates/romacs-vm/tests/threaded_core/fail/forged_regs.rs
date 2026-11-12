// A register capability made outside the core.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Pc, Regs};
    use crate::{Eng, Value};

    pub(super) fn body(e: Eng<'_, '_>, r: Regs, pc: Pc) -> Value {
        drop(r);
        go(e, Regs(std::ptr::null_mut()), pc.next())
    }
}
