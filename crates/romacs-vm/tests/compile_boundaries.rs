//! What the compiler enforces: the heap boundary and snapshot safety. Each
//! `fail/` program must not compile, and its `.stderr` pins WHICH error
//! rejects it, so an unrelated failure (a typo, a missing import) cannot pass
//! for the property; the plain tests here are the positive controls. A
//! `heap_boundary/fail/` program holds a `Local` across a call that can collect,
//! or reaches for the raw moving index. The rooting rule's are romacs-brand's own.
//!
//! No `pass` program for trybuild: one makes it `cargo build` this crate, where
//! with only failures it runs `cargo check`.
//!
//! One test for the whole crate: trybuild keeps one project per crate and
//! rewrites its manifest. A second test binary, run in parallel by nextest,
//! can take it over when trybuild's file lock looks stale under load; cargo
//! then builds the other's programs, and trybuild counts a program it has no
//! message for as compiled.

#[test]
fn the_compiler_enforces_the_boundaries() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/heap_boundary/fail/*.rs");
    t.compile_fail("tests/snapshot_safe/fail/*.rs");
}

/// Positive control: the sealed API is usable. Results are read inside one
/// borrow, Fixed values are kept across collections, and every evaluating
/// entry point is reachable.
#[test]
fn the_sealed_api_is_usable() {
    use romacs_vm::{Fixed, Vm};
    let mut vm = Vm::new();
    let kept = Fixed::integer(7).unwrap();
    vm.declare_special("hb-kept", kept);
    let text = vm.eval_source("(list 1 2 3)").unwrap().display().unwrap();
    assert_eq!(text, "(1 2 3)");
    vm.collect_garbage();
    assert_eq!(vm.eval_source("hb-kept").unwrap(), kept);
    let (car, cdr) = vm.eval_source("(cons 1 2)").unwrap().cons_pair().unwrap();
    assert_eq!((car.as_integer().unwrap(), cdr.as_integer().unwrap()), (1, 2));
    assert!(vm.eval_source("nil").unwrap().is_nil());
    let f = vm.compile(&romacs_vm::Form::Int(4)).unwrap();
    assert_eq!(vm.execute(f).unwrap().as_integer().unwrap(), 4);
    let read = vm.read_source("(a . b)").unwrap().map(|(v, end)| (v.display().unwrap(), end));
    assert_eq!(read, Some(("(a . b)".to_string(), 7)));
}

/// Data listed field by field and variant by variant is `SnapshotSafe`, and a
/// capture takes it as the embedder's carry.
#[test]
fn listed_data_is_snapshot_safe() {
    use romacs_vm::world::safe::{SnapshotSafe, snapshot_enum, snapshot_struct, snapshot_tuple};

    pub struct Report { name: String, rows: Vec<(u32, f64)>, note: Option<Box<str>> }
    snapshot_struct!(Report { name, rows, note });
    pub struct Pair(u8, Box<str>);
    snapshot_tuple!(Pair(a, b));
    #[expect(dead_code, reason = "listed for its type, and never built")]
    pub enum Shape { Dot, Line(u32), Area { width: u32, height: u32 } }
    snapshot_enum!(Shape { Shape::Dot => (), Shape::Line(n) => (n), Shape::Area { width, height } => (width, height) });

    fn carry<T: SnapshotSafe>() {}
    carry::<(Report, Pair, Shape)>();
    let _ = romacs_vm::world::image::capture::<Report>;
}
