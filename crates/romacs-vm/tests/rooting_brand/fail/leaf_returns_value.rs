// A heap value returned from a leaf into orchestration.
use romacs_vm::brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn bad<'b>(mut m: Mx<'_, 'b, Engine>, mut gc: Gc<'_, 'b>) -> u64 {
    let w = m.leaf((), |e, ()| Word(e.stack[0].0));
    collect(&mut m, gc.by_ref());
    w.0
}
fn main() {}
