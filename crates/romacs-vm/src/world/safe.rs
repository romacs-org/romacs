//! What a world image may hold (`romacs_snapshot`): the contract lives in
//! its own crate, so each crate whose types a VM holds lists them beside
//! their definitions.
pub use romacs_snapshot::{SnapshotSafe, require, snapshot_enum, snapshot_struct, snapshot_tuple};
