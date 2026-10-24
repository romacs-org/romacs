use romacs_face::{Datum, Remap};
fn main() {
    let raw = Datum::Nil;
    Remap::from_alist(&raw).unwrap();
}
