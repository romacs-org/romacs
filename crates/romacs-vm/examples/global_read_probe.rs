//! A warmed global-read loop for perf's externally controlled interval.
use romacs_vm::{Fixed, Vm};
use std::io::{self, BufRead, Write};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let symbols: usize = args[1].parse().unwrap();
    let constant = args.get(2).is_some_and(|s| s == "constant");
    let mut vm = Vm::new();
    // GC enabled with a raised threshold; report actual collections per interval.
    vm.set_collection_floor(usize::MAX / 4);
    vm.eval_source("(defvar redirect-bench-global 17) (defvar redirect-bench-local 1)").unwrap();
    for i in 0..symbols {
        vm.eval_source(&format!("(defvar redirect-padding-{i} 0)")).unwrap();
    }
    vm.eval_source("(make-local-variable 'redirect-bench-local)").unwrap();
    let mut lines = io::stdin().lock().lines();
    println!("READY"); io::stdout().flush().unwrap();
    while let Some(Ok(line)) = lines.next() {
        let n: usize = line.parse().unwrap();
        let read = if constant { "17" } else { "redirect-bench-global" };
        vm.eval_source(&format!("(defun redirect-bench-run () (let ((i 0) (answer 0)) (while (< i {n}) (setq answer {read}) (setq i (1+ i))) answer))")).unwrap();
        vm.eval_source("(redirect-bench-run)").unwrap();
        println!("READY"); io::stdout().flush().unwrap();
        lines.next().unwrap().unwrap();
        let before = vm.heap_stats().collections;
        let answer = vm.eval_source("(redirect-bench-run)").unwrap();
        assert_eq!(answer, Fixed::integer(17).unwrap());
        println!("DONE collections={}", vm.heap_stats().collections - before);
        io::stdout().flush().unwrap();
    }
}
