// A body that names its brand `'static`, so it could keep what it is
// given: the core's entries take only bodies generic over the brand.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Bytes, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body(e: Hot<'static, '_, '_>, r: Regs<'static>, pc: Pc<'static, Bytes>) -> Value {
        go(e, r, pc.next())
    }
}
