// A VM enters an image only as the primary, which capture validates: one
// inside the carry would go unchecked, with whatever resources it holds.
fn main() {
    let _ = romacs_vm::world::image::capture::<romacs_vm::Vm>;
    let _ = romacs_vm::world::image::capture::<(u32, Box<romacs_vm::Vm>)>;
}
