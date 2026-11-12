// An operand read through the successor, which may be the terminator.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Pc, Regs};
    use crate::{Eng, Value};

    pub(super) fn body(mut e: Eng<'_, '_>, mut r: Regs, pc: Pc) -> Value {
        let next = pc.next();
        r.window(&mut e)[pc.a()] = next.value();
        go(e, r, next)
    }
}
