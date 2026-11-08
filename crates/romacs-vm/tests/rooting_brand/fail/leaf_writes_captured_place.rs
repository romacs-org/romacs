// A heap value smuggled out of a leaf through a captured place.
use romacs_vm::brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn bad<'b>(mut m: Mx<'_, 'b, Engine>, mut gc: Gc<'_, 'b>) -> u64 {
    let mut slot = None;
    m.leaf((), |e, ()| { slot = Some(Word(e.stack[0].0)); });
    collect(&mut m, gc.by_ref());
    slot.unwrap().0
}
fn main() {}
