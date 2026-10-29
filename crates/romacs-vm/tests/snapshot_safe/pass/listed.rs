// Data listed field by field and variant by variant is SnapshotSafe, and a
// capture takes it as the embedder's carry.
use romacs_vm::world::safe::{SnapshotSafe, snapshot_enum, snapshot_struct, snapshot_tuple};

pub struct Report { name: String, rows: Vec<(u32, f64)>, note: Option<&'static str> }
snapshot_struct!(Report { name, rows, note });
pub struct Pair(u8, Box<str>);
snapshot_tuple!(Pair(a, b));
pub enum Shape { Dot, Line(u32), Area { width: u32, height: u32 } }
snapshot_enum!(Shape { Shape::Dot => (), Shape::Line(n) => (n), Shape::Area { width, height } => (width, height) });

fn carry<T: SnapshotSafe>() {}

fn main() {
    carry::<(Report, Pair, Shape)>();
    let _ = romacs_vm::world::image::capture::<Report>;
}
