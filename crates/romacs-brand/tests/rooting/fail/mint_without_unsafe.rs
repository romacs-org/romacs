// A permission minted without the key or unsafe.
use romacs_brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn bad() {
    let _gc: Gc<'_, '_> = Gc::unchecked();
}
fn main() {}
