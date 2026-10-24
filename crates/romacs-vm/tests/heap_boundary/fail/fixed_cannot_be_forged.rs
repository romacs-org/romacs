// A Fixed names only immediates and image references; its payload is private,
// so no private heap index can be smuggled through it.
fn main() {
    let _ = romacs_vm::Fixed(8);
}
