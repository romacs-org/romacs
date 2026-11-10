// One VM's permission used by another VM's orchestration.
use romacs_brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn bad<'b1, 'b2>(mut a: Mx<'_, 'b1, Engine>, gb: Gc<'_, 'b2>) {
    collect(&mut a, gb);
}
fn main() {}
