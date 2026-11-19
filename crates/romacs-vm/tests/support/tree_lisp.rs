//! The runtime tree's Lisp a test evaluates: the checkout's lisp/, read as
//! it is when the test runs, as a VM reads its own
//! (`romacs_vm::host::runtime_lisp`), never a copy compiled into the test.

/// The text of the runtime tree's lisp/PATH.
pub fn text(path: &str) -> String {
    let file = romacs_vm::host::runtime_lisp(path).unwrap();
    std::fs::read_to_string(&file).unwrap_or_else(|error| panic!("{}: {error}", file.display()))
}
