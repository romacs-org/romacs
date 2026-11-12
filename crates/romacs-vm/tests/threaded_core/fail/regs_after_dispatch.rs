// The frame's capability used after a dispatch consumed it.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Pc, Regs};
    use crate::{Eng, Value};

    pub(super) fn body(e: Eng<'_, '_>, mut r: Regs, pc: Pc) -> Value {
        let out = go(e, r, pc.next());
        let _ = &mut r;
        out
    }
}
