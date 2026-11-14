//! What a world image may hold (`romacs_snapshot`): the contract lives in
//! its own crate, so each crate whose types a VM holds lists them beside
//! their definitions. A listing is also its type's `Schema`, the walk a
//! census of an image takes (`world::image::census`).
pub use romacs_snapshot::{Schema, SnapshotSafe, Walk, snapshot_enum, snapshot_fields, snapshot_struct, snapshot_tuple};
