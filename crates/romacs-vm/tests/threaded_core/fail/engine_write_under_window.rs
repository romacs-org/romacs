// A mutable use of the engine while a window lives.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Pc, Regs};
    use crate::{Eng, Value};

    pub(super) fn body(mut e: Eng<'_, '_>, mut r: Regs, pc: Pc) -> Value {
        let regs = r.window(&mut e);
        e.budget -= 1;
        regs[pc.a()] = Value::NIL;
        go(e, r, pc.next())
    }
}
