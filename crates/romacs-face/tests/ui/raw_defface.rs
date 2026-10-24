use romacs_face::{Datum, Deffaces};
fn main() {
    let raw = Datum::Nil;
    Deffaces::default().defface("raw", &raw).unwrap();
}
