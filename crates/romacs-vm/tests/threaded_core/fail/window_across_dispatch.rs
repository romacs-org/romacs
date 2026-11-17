// A window held across the dispatch that hands its frame on.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f>) -> Value {
        let regs = r.window(&mut e);
        let out = go(e, r, pc.next());
        regs[pc.a()] = out;
        out
    }
}
