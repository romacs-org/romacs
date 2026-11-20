// Two steps in one handler: past the successor, where the terminator may be.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Bytes, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(e: Hot<'f, '_, '_>, r: Regs<'f>, pc: Pc<'f, Bytes>) -> Value {
        go(e, r, pc.next().next())
    }
}
