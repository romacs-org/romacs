// Two windows on one frame at once.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Pc, Regs};
    use crate::{Eng, Value};

    pub(super) fn body(mut e: Eng<'_, '_>, mut r: Regs, pc: Pc) -> Value {
        let regs = r.window(&mut e);
        let other = r.window(&mut e);
        regs[pc.a()] = other[pc.b()];
        go(e, r, pc.next())
    }
}
