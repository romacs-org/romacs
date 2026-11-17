// Sol's S2 probe: a register capability kept past its call, to be spent
// on a later one. Its brand is the call's, never `'static`.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, Hot, Pc, Regs};
    use crate::Value;
    use std::cell::Cell;

    thread_local! {
        static KEPT: Cell<Option<Pc<'static>>> = const { Cell::new(None) };
    }

    pub(super) fn body<'f>(e: Hot<'f, '_, '_>, r: Regs<'f>, pc: Pc<'f>) -> Value {
        KEPT.with(|kept| kept.set(Some(pc)));
        go(e, r, pc.next())
    }
}
