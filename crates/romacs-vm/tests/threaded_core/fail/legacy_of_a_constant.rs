// Sol's S5 probe: a constant's handler handing its instruction to a kernel
// it does not have. Only a kernel's fast path has a general path to take.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{legacy, Bytes, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(e: Hot<'f, '_, '_>, r: Regs<'f>, pc: Pc<'f, Bytes>) -> Value {
        legacy(e, r, pc)
    }
}
