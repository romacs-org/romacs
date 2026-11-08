// A leaf orchestrating (so collecting) while it holds a heap value: it has &mut E, never an Eng.
use romacs_vm::brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn body<'x, 'b>(_m: Mx<'x, 'b, Engine>, _gc: Gc<'_, 'b>, _: ()) -> u64 { 0 }
fn then(_e: Eng<'_, Engine>, n: u64) -> u64 { n }
fn bad<'b>(mut m: Mx<'_, 'b, Engine>) {
    m.leaf((), |e, ()| { orchestrate(e, (), body, then); });
}
fn main() {}
