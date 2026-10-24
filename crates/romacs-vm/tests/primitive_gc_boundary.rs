//! The three-tier primitive GC boundary (PRIMITIVE-DOCTRINE section 3).
//!
//! T1: a leaf body cannot reach the running VM (compile-fail probe below).
//! T2: a callback primitive's saved Values live in a traced continuation frame
//!     and are forwarded by collections that run inside the callback.
//! T3: a loop that calls Lisp is Lisp (`src/prims/prelude.el`), so its state
//!     is registers of ordinary traced frames.
use romacs_vm::Vm;
use std::path::Path;
use std::process::Command;

fn eval(vm: &mut Vm, source: &str) -> Result<String, String> {
    vm.eval_source(source).and_then(|v| v.display())
}

fn collecting_vm() -> Vm {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm
}

// Each callback runs past the 10,000-step budget, so the engine reaches a wait
// point inside it and, at collection floor 1, collects and compacts. The
// leading garbage puts every list built afterwards in a region that moves.
const CHURN: &str = "(let ((i 0)) (while (< i 12000) (setq i (1+ i)) (cons i i)))";
const GARBAGE: &str = "(let ((i 0)) (while (< i 3000) (setq i (1+ i)) (cons i i)))";

#[test]
fn t3_prelude_loops_call_lisp_and_survive_collections() {
    let mut vm = collecting_vm();
    // Read at run time, so a missing prelude is a failed assertion below.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/prims/prelude.el");
    let prelude = std::fs::read_to_string(path).unwrap_or_default();
    assert!(prelude.contains("(defun mapcar "), "src/prims/prelude.el must define mapcar");
    vm.eval_source(&prelude).expect("prelude evaluates");
    let before = vm.heap_stats().collections;
    let source = format!("{GARBAGE} (mapcar (lambda (x) {CHURN} (cons x (list x))) (list 1 2 3))");
    assert_eq!(eval(&mut vm, &source), Ok("((1 1) (2 2) (3 3))".into()));
    let collections = vm.heap_stats().collections - before;
    assert!(collections >= 3, "each callback must collect; saw {collections}");
    for (source, expected) in [
        ("(mapcar #'1+ [1 2 3])", "(2 3 4)"),
        ("(mapcar (lambda (c) c) \"ab\")", "(97 98)"),
        ("(mapcar #'1+ nil)", "nil"),
        // GNU mapcar1: a list the callback shortens ends the walk.
        ("(let ((l (list 1 2 3))) (mapcar (lambda (x) (setcdr l nil) x) l))", "(1)"),
        ("(condition-case e (mapcar #'1+ 5) (error e))", "(wrong-type-argument sequencep 5)"),
    ] {
        assert_eq!(eval(&mut vm, source), Ok(expected.into()), "{source}");
    }
}

/// Check this crate with the fixture `tests/fixtures/gc_boundary/probe.rs`
/// mounted inside `src/prims`, exactly where a declared primitive lives.
fn check_with_probe(probe: Option<&str>) -> (bool, String) {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("gc-boundary-probe");
    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    command
        .current_dir(manifest)
        .env("CARGO_TARGET_DIR", &target)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .args(["rustc", "--offline", "--lib", "--profile", "check", "--message-format", "short", "--"]);
    if let Some(probe) = probe {
        command.arg("--cfg").arg(format!("vm2_gc_probe=\"{probe}\""));
    }
    let output = command.output().expect("run cargo");
    (output.status.success(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn t1_leaf_bodies_cannot_reach_the_vm_or_use_unsafe() {
    // Control: the same command builds the crate without a probe mounted.
    let (ok, stderr) = check_with_probe(None);
    assert!(ok, "unprobed check must succeed:\n{stderr}");
    let (ok, stderr) = check_with_probe(Some("leaf_field"));
    assert!(!ok, "a leaf body reached Leaf's private VM field");
    assert!(
        stderr.contains("tests/fixtures/gc_boundary/probe.rs")
            && stderr.contains("error[E0616]: field `vm` of struct `")
            && stderr.contains("Leaf` is private"),
        "expected E0616 at the probe:\n{stderr}"
    );
    let (ok, stderr) = check_with_probe(Some("unsafe_body"));
    assert!(!ok, "a leaf body used unsafe code");
    assert!(
        stderr.contains("tests/fixtures/gc_boundary/probe.rs")
            && stderr.contains("usage of an `unsafe` block"),
        "expected forbid(unsafe_code) at the probe:\n{stderr}"
    );
}
