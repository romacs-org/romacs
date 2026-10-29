// A function pointer is an address in the builder's code.
use romacs_vm::world::safe::snapshot_struct;
pub struct Hook { run: fn() }
snapshot_struct!(Hook { run });
fn main() {}
