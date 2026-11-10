// A leaf starting a run (an Eng) with orchestration's permission.
use romacs_brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn bad<'b>(mut m: Mx<'_, 'b, Engine>, mut gc: Gc<'_, 'b>) {
    let lent = gc.by_ref();
    m.leaf((), move |e, ()| { let _run = Eng::lend(e, lent); });
}
fn main() {}
