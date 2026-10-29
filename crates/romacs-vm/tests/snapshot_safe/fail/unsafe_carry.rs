// The embedder's carry is held to the same contract as the VM.
fn main() {
    let _ = romacs_vm::world::image::capture::<fn()>;
}
