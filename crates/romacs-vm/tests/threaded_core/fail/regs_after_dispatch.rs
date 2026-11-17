// The frame's capability used after a dispatch consumed it.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Bytes, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Bytes>) -> Value {
        let out = go(e, r, pc.next());
        let _ = &mut r;
        out
    }
}
