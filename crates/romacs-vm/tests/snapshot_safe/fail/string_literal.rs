// A string literal is the builder's executable's data: another build has
// other bytes there. An image holds the text, or a name, instead.
#[derive(romacs_snapshot::Snapshot)]
pub struct Label { text: &'static str }
fn main() { let _ = Label { text: "builder" }; }
