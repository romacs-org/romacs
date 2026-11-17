// Control: a well-formed body compiles, so each other program here fails
// for its own misuse alone. The one error is this marker's, a lint the
// compiler reaches only when type and borrow checking found nothing.
include!("../support.rs");

#[forbid(unsafe_code)]
mod misuse {
    use crate::audited::{go, legacy, Hot, Pc, Regs};
    use crate::Value;

    pub(super) fn body<'f>(mut e: Hot<'f, '_, '_>, mut r: Regs<'f>, pc: Pc<'f>) -> Value {
        marker();
        let x = r.window(&mut e)[pc.d()];
        if e.pair(0).is_none() {
            return legacy(e, r, pc);
        }
        r.window(&mut e)[pc.a()] = x;
        go(e, r, pc.next())
    }

    #[deny(unused_variables)]
    fn marker() {
        let everything_above_compiled = ();
    }
}
