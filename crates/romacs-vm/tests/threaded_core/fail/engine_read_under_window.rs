// A shared read of the engine while a window lives.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Pc, Regs};
    use crate::{Eng, Value};

    pub(super) fn body(mut e: Eng<'_, '_>, mut r: Regs, pc: Pc) -> Value {
        let regs = r.window(&mut e);
        let budget = e.budget;
        regs[pc.a()] = Value(budget as u64);
        go(e, r, pc.next())
    }
}
