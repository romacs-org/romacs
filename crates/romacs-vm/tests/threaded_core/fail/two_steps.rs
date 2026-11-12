// Two steps in one handler: past the successor, where the terminator may be.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Pc, Regs};
    use crate::{Eng, Value};

    pub(super) fn body(e: Eng<'_, '_>, r: Regs, pc: Pc) -> Value {
        go(e, r, pc.next().next())
    }
}
