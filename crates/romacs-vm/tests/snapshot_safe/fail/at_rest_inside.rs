// A SnapshotSafe value may lie anywhere in an image, where no restore
// remakes an at-rest field: only an image's root (`snapshot_fields!`) has them.
use romacs_vm::world::safe::snapshot_struct;
pub struct Cache { table: std::collections::HashMap<u32, u32>, scratch: Vec<u8> }
snapshot_struct!(Cache { table } at_rest { scratch });
fn main() {}
