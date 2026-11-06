//! `thaw' is a bulk allocation that does not pass the single-object funnels:
//! its copies count in `memory-use-counts' as anything else consed does.
use romacs_vm::Vm;

/// The counter at INDEX before and after thawing what FROZEN evaluates to.
fn thawed(index: usize, frozen: &str) -> i64 {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let form = format!(
        "(let* ((f (freeze {frozen})) (a (nth {index} (memory-use-counts))) (keep (thaw f)))
           (- (nth {index} (memory-use-counts)) a))"
    );
    vm.eval_source(&form).unwrap().display().unwrap().parse().unwrap()
}

#[test]
fn thawed_conses_count() {
    assert!(thawed(0, "(make-list 1000 1)") >= 1000);
}

#[test]
fn thawed_vector_cells_count() {
    assert!(thawed(2, "(make-vector 1024 0)") >= 1024);
}

#[test]
fn thawed_strings_count_with_their_characters() {
    assert!(thawed(4, "(make-string 1024 ?a)") >= 1024);
    assert!(thawed(6, "(make-string 1024 ?a)") >= 1);
}

#[test]
fn a_thawed_table_counts_its_entries() {
    let empty = thawed(2, "(make-hash-table)");
    let filled = thawed(2, "(let ((h (make-hash-table))) (dotimes (i 1000) (puthash i i h)) h)");
    assert!(filled >= empty + 1000, "{empty} then {filled}");
}
