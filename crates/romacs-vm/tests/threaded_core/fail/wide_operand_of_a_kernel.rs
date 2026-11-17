// Sol's S4 probe: a kernel's fast path naming its wide operand B, which is
// the called symbol's slot, not a register. Its `Pc` offers no wide operand.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Hot, Kernel, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f, Kernel>) -> Value {
        *r.slot(&mut e, pc.wide_b()) = Value::NIL;
        go(e, r, pc.next())
    }
}
