// Every variant is matched: a variant added and not listed does not compile.
use romacs_vm::world::safe::snapshot_enum;
pub enum Grown { Old(u32), Added(u32) }
snapshot_enum!(Grown { Grown::Old(n) => (n) });
fn main() {}
