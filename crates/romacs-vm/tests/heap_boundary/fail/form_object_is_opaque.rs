// The hidden expansion payload cannot be read outside the crate.
fn main() {
    let form = romacs_vm::Form::Int(1);
    if let romacs_vm::Form::Object(object) = form {
        let _ = object.0;
    }
}
