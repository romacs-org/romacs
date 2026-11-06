//! The pool's memory sampler (crates/romacs-vm/src/isolate.rs): a thread
//! that ends with the VM that started it. Alone in its binary, so its
//! threads are the only ones counted.
use romacs_vm::Vm;

fn samplers() -> usize {
    std::fs::read_dir("/proc/self/task").unwrap().flatten()
        .filter(|task| std::fs::read_to_string(task.path().join("comm")).is_ok_and(|name| name.trim() == "romacs-pool-mem"))
        .count()
}

fn settles(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        if check() { return true; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

#[test]
fn the_sampler_ends_with_its_vm() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(romacs--isolate-pool-memory)").unwrap();
    // The thread names itself as it starts.
    assert!(settles(|| samplers() == 1), "no sampler started");
    drop(vm);
    assert!(settles(|| samplers() == 0), "the sampler outlived its VM");
}
