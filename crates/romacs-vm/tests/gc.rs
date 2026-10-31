use romacs_vm::{Form, ImageRegion, Vm};
fn form(s: &str) -> Form {
    Form::from_json(serde_json::from_str(s).unwrap()).unwrap()
}
#[test]
fn allocation_far_exceeds_retained_heap() {
    let mut vm = Vm::new();
    vm.set_collection_floor(16384);
    let f = vm.compile(&form(r#"["let",[["i",0]],["while",["<","i",1000000],["cons","i","nil"],["setq","i",["1+","i"]]],"i"]"#)).unwrap();
    assert_eq!(vm.execute(f).unwrap().as_integer().unwrap(), 1000000);
    assert!(
        vm.heap_objects() < 25000,
        "unreclaimed heap: {}",
        vm.heap_objects()
    );
    assert_eq!(vm.heap_stats().allocations, 1000000);
    assert!(vm.heap_stats().collections > 20);
    vm.collect_garbage();
    assert_eq!(vm.heap_stats().reclaimed, 1000000);
    assert_eq!(vm.heap_stats().live_bytes, 0);
    assert!(vm.heap_stats().capacity_bytes <= 16384 * 16);
}
#[test]
fn moving_roots_preserve_closures_cells_globals_and_caches() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let f = vm.compile(&form(r#"["progn",["cons",0,"nil"],["setq","keeper",["let",[["x",["cons",42,"nil"]]],["lambda",[],["car","x"]]]],["let",[["i",0]],["while",["<","i",50000],["cons","i","nil"],["funcall","keeper"],["setq","i",["1+","i"]]]],["funcall","keeper"]]"#)).unwrap();
    assert_eq!(vm.execute(f).unwrap().as_integer().unwrap(), 42);
    vm.collect_garbage();
    let f = vm.compile(&form(r#"["funcall","keeper"]"#)).unwrap();
    assert_eq!(vm.execute(f).unwrap().as_integer().unwrap(), 42);
    assert!(vm.heap_stats().collections > 0);
}
#[test]
fn explicit_external_roots_are_rewritten() {
    let mut vm = Vm::new();
    let f = vm
        .compile(&form(r#"["progn",["cons",0,"nil"],["cons",42,"nil"]]"#))
        .unwrap();
    let root = vm.execute(f).unwrap().root();
    vm.collect_garbage();
    assert_eq!(vm.heap_objects(), 1);
    assert_eq!(root.get(&vm).unwrap().cons_pair().unwrap().0.as_integer().unwrap(), 42);
}
#[test]
fn embedder_roots_survive_automatic_gc_and_error_return() {
    let mut vm = Vm::new();
    let make = vm
        .compile(&form(r#"["progn",["cons",0,"nil"],["cons",42,"nil"]]"#))
        .unwrap();
    let root = vm.execute(make).unwrap().root();
    for fail in [false, true] {
        let mut body = form(
            r#"["progn",["let",[["i",0]],["while",["<","i",50000],["cons","i","nil"],["setq","i",["1+","i"]]]],42]"#,
        );
        if fail && let Form::List(ref mut list) = body {
            *list.last_mut().unwrap() = form(r#"["car",1]"#);
        }
        let f = vm.compile(&body).unwrap();
        let outcome = vm.execute(f).map(|_| ());
        assert_eq!(outcome.is_err(), fail);
        assert_eq!(root.get(&vm).unwrap().cons_pair().unwrap().0.as_integer().unwrap(), 42);
    }
    drop(root);
    vm.collect_garbage();
    assert_eq!(vm.heap_objects(), 0);
}
#[test]
fn long_generic_primitive_chains_keep_bounded_native_stack() {
    let mut vm = Vm::new();
    let f = vm.compile(&form(r#"["let",[["l",["cons",42,"nil"]],["i",0]],["while",["<","i",300000],["car","l"],["setq","i",["1+","i"]]],"i"]"#)).unwrap();
    assert_eq!(vm.execute(f).unwrap().as_integer().unwrap(), 300000);
}
#[test]
fn idle_literals_and_entry_cache_copies_survive_compaction() {
    let mut vm = Vm::new();
    let dead = vm
        .compile(&form(r#"["progn",["vector",0],["lambda",[],0]]"#))
        .unwrap();
    vm.execute(dead).unwrap();
    let install = vm.compile(&form(r#"["progn",["defun","literal",[],["quote",{"vector":[{"string":"λ"},[42]]}]],["defun","caller",[],["literal"]]]"#)).unwrap();
    vm.execute(install).unwrap();
    let call = vm.compile(&form(r#"["caller"]"#)).unwrap();
    let before = vm.execute(call).unwrap();
    let expected = before.display().unwrap();
    assert!(vm.heap_stats().allocations > 0);
    vm.collect_garbage();
    assert!(vm.heap_stats().reclaimed >= 2);
    // Reuse the exact old executable and its already-warm entry caches.
    let after = vm.execute(call).unwrap();
    assert_eq!(after.display().unwrap(), expected);
    let drop = vm
        .compile(&form(r#"["progn",["defun","literal",[],99],["caller"]]"#))
        .unwrap();
    assert_eq!(vm.execute(drop).unwrap().as_integer().unwrap(), 99);
    vm.collect_garbage();
    assert_eq!(vm.execute(call).unwrap().as_integer().unwrap(), 99);
}
#[test]
fn read_only_graph_is_shared_while_worker_heaps_collect() {
    use std::{fs::OpenOptions, io::Write};
    let path = std::env::temp_dir().join(format!("vm2-image-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    // Record 0 -> record 1 -> nil; not a permanent leaf.
    let words: [u64; 4] = [(7 << 3) | 1, (1 << 63) | (1 << 3) | 3, (42 << 3) | 1, 0];
    for w in words {
        file.write_all(&w.to_le_bytes()).unwrap();
    }
    file.flush().unwrap();
    let image = unsafe { ImageRegion::map(&file) }.unwrap();
    std::fs::remove_file(path).unwrap();
    drop(file);
    assert!(image.reference(2).is_err());
    let threads: Vec<_> = (0..2).map(|_| {
        let image = image.clone();
        std::thread::spawn(move || {
            let mut vm = Vm::new();
            vm.attach_image(image.clone()).unwrap();
            vm.declare_special("world", image.reference(0).unwrap());
            vm.set_collection_floor(1);
            let f = vm.compile(&form(r#"["let",[["i",0]],["while",["<","i",50000],["cons","i","world"],["setq","i",["1+","i"]]],["car",["cdr","world"]]]"#)).unwrap();
            assert_eq!(vm.execute(f).unwrap().as_integer().unwrap(), 42);
            vm.collect_garbage();
            assert_eq!(vm.heap_objects(), 0);
            let f = vm.compile(&form(r#"["nreverse","world"]"#)).unwrap();
            assert!(vm.execute(f).unwrap_err().contains("Attempt to modify read-only object"));
        })
    }).collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
