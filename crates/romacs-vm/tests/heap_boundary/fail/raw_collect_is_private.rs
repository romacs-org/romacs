// Only a rootless collection is public: there is no API to hand the
// collector a slice of raw values.
fn main() {
    let mut vm = romacs_vm::Vm::new();
    vm.collect(&mut []);
}
