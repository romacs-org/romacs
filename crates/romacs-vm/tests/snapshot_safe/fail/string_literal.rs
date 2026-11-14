// A string literal is the builder's executable's data: another build has
// other bytes there. An image holds the text, or a name, instead.
use romacs_vm::world::safe::snapshot_struct;
pub struct Label { text: &'static str }
snapshot_struct!(Label { text });
fn main() { let _ = Label { text: "builder" }; }
