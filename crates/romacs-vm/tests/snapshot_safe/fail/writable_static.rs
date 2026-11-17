// A reference to a writable static names the builder's static, not this
// process's.
static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[derive(romacs_snapshot::Snapshot)]
pub struct Counter { count: &'static std::sync::atomic::AtomicUsize }
fn main() { let _ = Counter { count: &COUNT }; }
