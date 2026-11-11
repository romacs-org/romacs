// A permission-less body handed a permission as its argument.
use romacs_brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn bad<'b>(e: &mut Engine, gc: Gc<'_, 'b>) {
    without_gc(e, gc, |mut m, gc| collect(&mut m, gc));
}
fn main() {}
