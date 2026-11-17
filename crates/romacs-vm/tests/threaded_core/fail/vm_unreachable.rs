// Sol's S1 probe: a body that frees the register file it runs in. The
// engine a typed handler holds offers its budget and conses alone.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(mut e: Hot<'f, '_, '_>, r: Regs<'f>, pc: Pc<'f>) -> Value {
        e.vm.stack.clear();
        go(e, r, pc.next())
    }
}
