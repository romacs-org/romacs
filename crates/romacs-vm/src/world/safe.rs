//! What a world image may hold (`romacs_snapshot`): the contract lives in
//! its own crate, and each crate whose types a VM holds derives it on their
//! definitions (`#[derive(romacs_snapshot::Snapshot)]`). The derive is also
//! the type's `Schema`, the walk a census of an image takes
//! (`world::image::census`).
pub use romacs_snapshot::{Schema, Snapshot, SnapshotSafe, Walk};
