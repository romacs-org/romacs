// A permission-less body that brings an outer permission in to collect.
use romacs_brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn bad<'b>(e: &mut Engine, gc: Gc<'_, 'b>) {
    without_gc(e, (), |mut m, ()| collect(&mut m, gc));
}
fn main() {}
