//! Big strings made and dropped in a loop are collected as they are made,
//! as GNU charges a string's bytes to the next collection (alloc.c
//! consing_until_gc): copies of a 1 MB buffer, with and without a process
//! wait between them (tramp's wait-for-regexp copies its connection buffer
//! between waits), make collections in step with the work and keep memory
//! flat from N copies to 4N.
use romacs_vm::Vm;

const N: usize = 300;

fn rss_kb() -> usize {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
    line.split_whitespace().nth(1).unwrap().parse().unwrap()
}

/// Collections made while running LOOP (a form of `n') at N and at 4N, and
/// the resident memory after each.
fn copies(body: &str) -> [(i64, usize); 2] {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(progn (set-buffer (get-buffer-create \"gc-payload\")) (insert (make-string (* 1024 1024) ?x)))").unwrap();
    [N, 4 * N].map(|n| {
        let form = format!("(let ((gcs gcs-done)) (dotimes (_ {n}) {body}) (- gcs-done gcs))");
        let collections = vm.eval_source(&form).unwrap().display().unwrap().parse().unwrap();
        (collections, rss_kb())
    })
}

fn assert_collected(body: &str) {
    let [(few, rss_n), (many, rss_4n)] = copies(body);
    // Each copy is over `gc-cons-threshold' (800000 bytes): GNU collects at
    // about every other copy.
    assert!(few as usize >= N / 4, "{body}: {few} collections over {N} copies");
    assert!(many >= 3 * few, "{body}: {many} collections over {} copies, {few} over {N}", 4 * N);
    assert!(rss_4n < rss_n + 128 * 1024, "{body}: resident {rss_n} kB at {N} copies, {rss_4n} kB at {}", 4 * N);
}

#[test]
fn copies_of_a_big_buffer_are_collected_as_they_are_made() {
    assert_collected("(buffer-string)");
}

#[test]
fn copies_between_process_waits_are_collected_as_they_are_made() {
    assert_collected("(progn (buffer-substring (point-min) (point-max)) (accept-process-output nil 0))");
}
