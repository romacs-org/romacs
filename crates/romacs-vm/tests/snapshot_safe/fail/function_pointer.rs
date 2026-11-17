// A function pointer is an address in the builder's code.
#[derive(romacs_snapshot::Snapshot)]
pub struct Hook { run: fn() }
fn main() {}
