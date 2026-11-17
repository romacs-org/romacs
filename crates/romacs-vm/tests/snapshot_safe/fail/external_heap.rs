// A raw pointer may address memory the arena does not own: C's heap, a
// library's, the stack.
#[derive(romacs_snapshot::Snapshot)]
pub struct Foreign(*mut u8, std::ptr::NonNull<u8>);
fn main() {}
