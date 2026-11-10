// The permission captured into a leaf, to collect inside it.
use romacs_brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn bad<'b>(mut m: Mx<'_, 'b, Engine>, mut gc: Gc<'_, 'b>) {
    let lent = gc.by_ref();
    m.leaf((), move |e, ()| { let _held = &e.stack[0]; drop(lent); });
}
fn main() {}
