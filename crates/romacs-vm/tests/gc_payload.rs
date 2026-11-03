//! Big payloads made and dropped in a loop are collected as they are made,
//! as GNU charges a string's bytes and a vector's slots to the next
//! collection (alloc.c consing_until_gc) and checks at every call
//! (maybe_gc): copies of a 1 MB buffer, with and without a process wait
//! between them (tramp's wait-for-regexp copies its connection buffer
//! between waits), strings made in a nested `eval', bool vectors and thawed
//! frozen strings make collections in step with the work and keep memory
//! flat from N to 4N. A big vector the heap keeps, which every collection
//! traces, paces collection by its size.
use romacs_vm::Vm;

const N: usize = 300;

fn rss_kb() -> usize {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
    line.split_whitespace().nth(1).unwrap().parse().unwrap()
}

fn vm(setup: &str) -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(progn (set-buffer (get-buffer-create \"gc-payload\")) (insert (make-string (* 1024 1024) ?x)))").unwrap();
    vm.eval_source(setup).unwrap();
    vm
}

/// Collections BODY makes run N times.
fn collections(vm: &mut Vm, body: &str, n: usize) -> i64 {
    let form = format!("(let ((gcs gcs-done)) (dotimes (_ {n}) {body}) (- gcs-done gcs))");
    vm.eval_source(&form).unwrap().display().unwrap().parse().unwrap()
}

/// BODY, which makes about 1 MB of garbage, collects at least every EVERY
/// runs at N and at 4N, and resident memory stays flat between them.
fn assert_collected(setup: &str, body: &str, every: usize) {
    let mut vm = vm(setup);
    let few = collections(&mut vm, body, N);
    let rss_n = rss_kb();
    let many = collections(&mut vm, body, 4 * N);
    let rss_4n = rss_kb();
    assert!(few as usize >= N / every, "{body}: {few} collections over {N} runs");
    assert!(many >= 3 * few, "{body}: {many} collections over {} runs, {few} over {N}", 4 * N);
    assert!(rss_4n < rss_n + 128 * 1024, "{body}: resident {rss_n} kB after {N} runs, {rss_4n} kB after {}", 4 * N);
}

#[test]
fn copies_of_a_big_buffer_are_collected_as_they_are_made() {
    assert_collected("nil", "(buffer-string)", 4);
}

#[test]
fn copies_between_process_waits_are_collected_as_they_are_made() {
    assert_collected("nil", "(progn (buffer-substring (point-min) (point-max)) (accept-process-output nil 0))", 4);
}

#[test]
fn strings_made_in_a_nested_eval_are_collected_as_they_are_made() {
    assert_collected("nil", "(eval '(make-string 1048576 ?x))", 4);
}

#[test]
fn bool_vectors_are_collected_as_they_are_made() {
    assert_collected("nil", "(make-bool-vector 8388608 nil)", 4);
}

/// The frozen 1 MB string is live heap too (`region_weight'), which the
/// proportional rule paces by: a collection every dozen or so thaws.
#[test]
fn thawed_strings_are_collected_as_they_are_made() {
    assert_collected("(defvar gc-payload-frozen (freeze (make-string 1048576 ?x)))", "(thaw gc-payload-frozen)", 32);
}

/// A kept vector of ten million slots, which every collection traces,
/// counts in the live heap the proportional rule paces by: 1 MB strings
/// beside it do not collect, and trace it, at each one, as they would if it
/// counted as one object (VM2-GC-PACING scales GNU's rule).
#[test]
fn a_big_kept_vector_paces_collection_by_its_size() {
    let mut vm = vm("(progn (defvar gc-payload-kept (make-vector 10000000 nil)) (garbage-collect))");
    let made = collections(&mut vm, "(make-string 1048576 ?x)", N);
    assert!((made as usize) <= N / 8, "{made} collections over {N} strings beside a ten-million-slot vector");
}
