// A reference to a writable static names the builder's static, not this
// process's.
use romacs_vm::world::safe::snapshot_struct;
static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub struct Counter { count: &'static std::sync::atomic::AtomicUsize }
snapshot_struct!(Counter { count });
fn main() { let _ = Counter { count: &COUNT }; }
