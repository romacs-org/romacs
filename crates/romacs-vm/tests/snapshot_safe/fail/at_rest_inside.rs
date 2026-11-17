// A SnapshotSafe value may lie anywhere in an image, where no restore
// remakes an at-rest field: only an image's root (`#[snapshot(root)]`) has them.
#[derive(romacs_snapshot::Snapshot)]
pub struct Cache {
    table: std::collections::HashMap<u32, u32>,
    #[snapshot(at_rest)]
    scratch: Vec<u8>,
}
fn main() {}
