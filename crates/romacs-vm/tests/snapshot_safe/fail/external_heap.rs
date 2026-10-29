// A raw pointer may address memory the arena does not own: C's heap, a
// library's, the stack.
use romacs_vm::world::safe::snapshot_tuple;
pub struct Foreign(*mut u8, std::ptr::NonNull<u8>);
snapshot_tuple!(Foreign(raw, nonnull));
fn main() {}
