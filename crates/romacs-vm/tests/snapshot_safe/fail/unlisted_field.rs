// Every field is listed: a field added and not listed does not compile.
use romacs_vm::world::safe::snapshot_struct;
pub struct Grown { old: u32, added: u32 }
snapshot_struct!(Grown { old });
fn main() {}
