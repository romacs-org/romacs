// A window held across the dispatch that hands its frame on.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Pc, Regs};
    use crate::{Eng, Value};

    pub(super) fn body(mut e: Eng<'_, '_>, mut r: Regs, pc: Pc) -> Value {
        let regs = r.window(&mut e);
        let out = go(e, r, pc.next());
        regs[pc.a()] = out;
        out
    }
}
