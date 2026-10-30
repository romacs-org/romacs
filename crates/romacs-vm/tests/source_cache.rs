use romacs_vm::{Fixed, Vm};

#[test]
fn tiny_tier_is_direct_but_redefinitions_are_observed() {
    let mut vm = Vm::new();
    for _ in 0..3 {
        assert_eq!(vm.eval_source("(abs -2)").unwrap().as_integer().unwrap(), 2);
    }
    assert_eq!(vm.source_cache_stats().direct, 3);
    assert_eq!(vm.source_cache_stats().compilations, 0);
    vm.eval_source("(defun abs (x) (- x 1))").unwrap();
    assert_eq!(vm.eval_source("(abs -2)").unwrap().as_integer().unwrap(), -3);
    // A byte-op call is the primitive, as compiled GNU code (VM2-SEALED1)...
    vm.eval_source("(defun + (x y) (- x y))").unwrap();
    assert_eq!(vm.eval_source("(+ 8 3)").unwrap().as_integer().unwrap(), 11);
    // ...except that a macro, or an alias of one, expands first (bytecomp
    // macroexpands before open-coding): pinned compiled GNU gives 42.
    vm.eval_source("(defmacro sc-m (a b) 42) (defalias '+ 'sc-m)").unwrap();
    assert_eq!(vm.eval_source("(+ 8 3)").unwrap().as_integer().unwrap(), 42);
}
#[test]
fn mutable_literals_are_not_reused_as_source_constants() {
    let mut vm = Vm::new();
    let source = "(let ((s (copy-sequence \"abc\"))) (aset s 0 ?x) s)";
    for _ in 0..3 {
        let v = vm.eval_source(source).unwrap();
        assert_eq!(v.display().unwrap(), "\"xbc\"");
    }
    let source = "(let ((s '(1 2))) (setcar s (1+ (car s))) (car s))";
    for _ in 0..3 {
        assert_eq!(vm.eval_source(source).unwrap().as_integer().unwrap(), 2);
    }
}
#[test]
fn special_status_and_new_macro_invalidate_cached_ir() {
    let mut vm = Vm::new();
    let source = "(let ((x 2)) (funcall (lambda () x)))";
    assert_eq!(vm.eval_source(source).unwrap().as_integer().unwrap(), 2);
    vm.declare_special("x", Fixed::integer(9).unwrap());
    assert_eq!(vm.eval_source(source).unwrap().as_integer().unwrap(), 2);
    assert!(vm.source_cache_stats().invalidations > 0);
    vm.eval_source("(defun future-m () 3)").unwrap();
    assert_eq!(
        vm.eval_source("(future-m)").unwrap().as_integer().unwrap(),
        3
    );
    vm.eval_source("(defmacro future-m () 42)").unwrap();
    assert_eq!(
        vm.eval_source("(future-m)").unwrap().as_integer().unwrap(),
        42
    );
}
#[test]
fn prepared_arguments_gc_and_macro_redefinition() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.eval_source("(defmacro handle-m () 3)").unwrap();
    let h = vm.prepare_source("(lambda (x) (+ x (handle-m)))").unwrap();
    assert_eq!(
        vm.invoke_prepared(h, &[Fixed::integer(2).unwrap().into()])
            .unwrap()
            .as_integer()
            .unwrap(),
        5
    );
    vm.eval_source("(defmacro handle-m () 40)").unwrap();
    assert_eq!(
        vm.invoke_prepared(h, &[Fixed::integer(2).unwrap().into()])
            .unwrap()
            .as_integer()
            .unwrap(),
        42
    );
    let h = vm
        .prepare_source(
            "(lambda (x) (let ((i 0)) (while (< i 50000) (cons i nil) (setq i (1+ i))) x))",
        )
        .unwrap();
    vm.read_source("[unreachable]").unwrap();
    let arg = vm.read_source("[1.25 λ]").unwrap().unwrap().0.root();
    let v = vm.invoke_prepared(h, &[(&arg).into()]).unwrap();
    assert_eq!(v.display().unwrap(), "[1.25 λ]");
    assert!(vm.heap_stats().collections > 0);
    assert!(vm.invoke_prepared(h, &[]).is_err());
    assert!(vm.prepared_ir(h).unwrap().contains("return r"));
}
#[test]
fn persistent_images_are_deterministic_relocatable_and_fail_closed() {
    let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(format!(
        ".hermes/cache/scratch/romacs/vm2/cache-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = "(let ((x 20)) (funcall (lambda (y) (+ x y)) 22))";
    let mut vm = Vm::new();
    vm.set_source_cache_store(dir.clone()).unwrap();
    assert_eq!(vm.eval_source(source).unwrap().as_integer().unwrap(), 42);
    let indices: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("source-")
        })
        .collect();
    assert_eq!(indices.len(), 1);
    let hash = std::fs::read_to_string(&indices[0]).unwrap();
    let mut warm = Vm::new();
    warm.read_source("slot-relocation-noise").unwrap();
    warm.set_source_cache_store(dir.clone()).unwrap();
    assert_eq!(warm.eval_source(source).unwrap().as_integer().unwrap(), 42);
    assert_eq!(warm.source_cache_stats().disk_hits, 1);
    assert_eq!(warm.source_cache_stats().compilations, 0);
    warm.collect_garbage();
    assert_eq!(warm.eval_source(source).unwrap().as_integer().unwrap(), 42);
    // A bad hint must compile rather than installing unverified IR.
    std::fs::write(&indices[0], "invalid").unwrap();
    let mut cold = Vm::new();
    cold.read_source("different-session").unwrap();
    cold.set_source_cache_store(dir.clone()).unwrap();
    assert_eq!(cold.eval_source(source).unwrap().as_integer().unwrap(), 42);
    assert_eq!(cold.source_cache_stats().disk_hits, 0);
    assert_eq!(
        std::fs::read_to_string(&indices[0]).unwrap(),
        hash,
        "same source and semantics must publish identical portable IR"
    );
    cold.eval_source("(eval-cache-clear)").unwrap();
    assert!(!indices[0].exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prepared_literals_are_shared_without_recompilation_and_dynamic_macros_recompile() {
    let mut vm = Vm::new();
    let h = vm.prepare_source("(lambda () '(1 2))").unwrap();
    let first = format!("{:?}", vm.invoke_prepared(h, &[]).unwrap());
    let compiled = vm.source_cache_stats().compilations;
    let second = format!("{:?}", vm.invoke_prepared(h, &[]).unwrap());
    assert_eq!(first, second);
    assert_eq!(vm.source_cache_stats().compilations, compiled);
    vm.eval_source("(setq macro-value 3) (defmacro state-m () macro-value)")
        .unwrap();
    assert_eq!(
        vm.eval_source("(state-m)").unwrap().as_integer().unwrap(),
        3
    );
    vm.eval_source("(setq macro-value 42)").unwrap();
    assert_eq!(
        vm.eval_source("(state-m)").unwrap().as_integer().unwrap(),
        42
    );
    assert!(vm.eval_source("(funcall #'state-m)").is_err());
}

#[test]
fn generated_bindings_record_their_special_status() {
    let mut vm = Vm::new();
    vm.eval_source("(defmacro make-held () '(let ((generated 1)) (lambda () generated)))")
        .unwrap();
    vm.eval_source("(setq held (make-held))").unwrap();
    assert_eq!(
        vm.eval_source("(funcall held)")
            .unwrap()
            .as_integer()
            .unwrap(),
        1
    );
    vm.declare_special("generated", Fixed::integer(9).unwrap());
    vm.eval_source("(setq held (make-held))").unwrap();
    assert_eq!(
        vm.eval_source("(funcall held)")
            .unwrap()
            .as_integer()
            .unwrap(),
        9
    );
}

#[test]
fn macro_redefinition_invalidates_only_dependent_source() {
    let mut vm = Vm::new();
    vm.eval_source("(defmacro cached-m () '(+ 1 2))").unwrap();
    let v = vm.eval_source("(cached-m)").unwrap();
    assert_eq!(v.as_integer().unwrap(), 3);
    vm.eval_source("(let ((x 8)) (+ x 9))").unwrap();
    let before = vm.source_cache_stats();
    vm.eval_source("(defmacro cached-m () '(+ 20 22))").unwrap();
    let v = vm.eval_source("(cached-m)").unwrap();
    assert_eq!(
        v.as_integer().unwrap(),
        42,
        "stale macro expansion returned"
    );
    let after = vm.source_cache_stats();
    assert!(after.compilations > before.compilations);
    assert!(after.invalidations > before.invalidations);
    let hits = after.hits;
    vm.eval_source("(let ((x 8)) (+ x 9))").unwrap();
    assert_eq!(vm.source_cache_stats().hits, hits + 1);
}
