// A request's value kept for use after the next one, which could collect it.
use romacs_brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn bad<'b>(mut m: Mx<'_, 'b, Engine>, mut gc: Gc<'_, 'b>) -> u64 {
    let binding = request(&mut m, gc.by_ref(), (), |_, (), value| value);
    request(&mut m, gc.by_ref(), (), |_, (), _| ());
    binding.0
}
fn main() {}
