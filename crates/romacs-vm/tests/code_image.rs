use romacs_vm::{Form, ImageRegion, Vm};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Store(PathBuf);
impl Store {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "vm2-code-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self, key: &str) -> Arc<ImageRegion> {
        unsafe { ImageRegion::open(&self.0, key) }.unwrap()
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn compile(vm: &mut Vm, j: serde_json::Value) -> usize {
    vm.compile(&Form::from_json(j).unwrap()).unwrap()
}
fn word(bytes: &[u8], at: usize) -> usize {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
}
fn body_at(bytes: &[u8], index: usize) -> usize {
    let conses = word(bytes, 8);
    let strings = word(bytes, 16);
    64 + (conses + strings) * 16 + word(bytes, 64 + (conses + index) * 16)
}
fn corrupt(store: &Path, key: &str, at: usize) {
    use std::os::unix::fs::PermissionsExt;
    let path = store.join(key);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[at] ^= 2;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(&path, bytes).unwrap();
}

#[test]
fn bare_vm_startup_allocates_no_lisp_objects() {
    let vm = Vm::new();
    let stats = vm.heap_stats();
    assert_eq!((stats.live_objects, stats.live_bytes), (0, 0), "BARE_VM_ZERO_ALLOCATION");
}

#[test]
fn raw_data_image_attachment_allocates_no_lisp_objects() {
    let store = Store::new();
    let mut builder = romacs_vm::ImageBuilder::default();
    builder.string("raw data");
    let key = builder.publish(&store.0, [60; 32]).unwrap();
    let mut vm = Vm::new();
    vm.attach_image(store.open(&key)).unwrap();
    let stats = vm.heap_stats();
    assert_eq!((stats.live_objects, stats.live_bytes), (0, 0), "RAW_DATA_IMAGE_ZERO_ALLOCATION");
}

#[test]
fn image_top_level_defvar_yank_menu_remains_mutable() {
    let store = Store::new();
    let mut source = Vm::new();
    source.eval_source("(defvar freeze-menu '(\"Select Yank\" nil)) (setq freeze-alias freeze-menu) (fset 'freeze-menu-map (cons 'keymap freeze-menu))").unwrap();
    let entry = source.compile_source("nil").unwrap();
    let key = source.publish_code_image(&store.0, entry, [61; 32]).unwrap();
    let image = store.open(&key);
    let (mut loaded, _) = Vm::from_code_image(image.clone(), [61; 32]).unwrap();
    let startup = loaded.heap_stats();
    eprintln!("YANK_MENU_STARTUP: {} objects / {} bytes", startup.live_objects, startup.live_bytes);
    assert_eq!((startup.live_objects, startup.live_bytes), (4, 195), "EXECUTABLE_IMAGE_RESTORE_ALLOCATION_RATCHET");
    assert_eq!(loaded.image_functions_materialized(), 0, "YANK_MENU_COLD_CODE");
    loaded.collect_garbage();
    let result = loaded.eval_source("(setcdr (cdr freeze-menu) (cons '(entry . 7) nil)) (list (eq freeze-menu freeze-alias) (eq freeze-menu (cdr (symbol-function 'freeze-menu-map))) freeze-menu)");
    assert!(result.is_ok(), "YANK_MENU_MUTABLE: top-level shared defvar literal must support in-place setcdr: {result:?}");
    assert_eq!(result.unwrap().display().unwrap(), "(t t (\"Select Yank\" nil (entry . 7)))", "YANK_MENU_ALIAS_IDENTITY");
}

#[test]
fn image_mixed_code_data_identity_and_strand_isolation() {
    let store = Store::new();
    let mut source = Vm::new();
    source.eval_source("(defun freeze-mixed () '(1 2))").unwrap();
    source.eval_source("(defvar freeze-value (freeze-mixed)) (setplist 'freeze-owner freeze-value)").unwrap();
    let entry = source.compile_source("(freeze-mixed)").unwrap();
    let key = source.publish_code_image(&store.0, entry, [64; 32]).unwrap();
    let image = store.open(&key);
    let (mut first, _) = Vm::from_code_image(image.clone(), [64; 32]).unwrap();
    let (mut second, _) = Vm::from_code_image(image, [64; 32]).unwrap();
    assert_eq!(first.image_functions_materialized(), 0);
    first.collect_garbage();
    assert_eq!(first.eval_source("(list (eq freeze-value (freeze-mixed)) (eq freeze-value (symbol-plist 'freeze-owner)))").unwrap().display().unwrap(), "(t t)", "MIXED_CODE_DATA_IDENTITY");
    first.eval_source("(setcar freeze-value 9)").unwrap();
    first.collect_garbage();
    assert_eq!(first.eval_source("(freeze-mixed)").unwrap().display().unwrap(), "(9 2)", "MIXED_MUTATION_VISIBLE_IN_CODE");
    assert_eq!(second.eval_source("(freeze-mixed)").unwrap().display().unwrap(), "(1 2)", "STRAND_PRIVATE_MUTATION");
}

#[test]
fn image_literal_mutation_is_private_and_execution_continues() {
    let store = Store::new();
    let mut source = Vm::new();
    let install = compile(&mut source, json!(["defun", "freeze-body", [], ["nreverse", ["quote", [1, 2]]]]));
    source.execute(install).unwrap();
    let entry = source.compile_source("nil").unwrap();
    let key = source.publish_code_image(&store.0, entry, [62; 32]).unwrap();
    let (mut loaded, _) = Vm::from_code_image(store.open(&key), [62; 32]).unwrap();
    let mutation = loaded.eval_source("(freeze-body)").map(|value| value.display().unwrap());
    assert_eq!(mutation, Ok("(2 1)".into()), "CODE_LITERAL_PRIVATE_MUTATION");
    assert_eq!(loaded.eval_source("(condition-case e (freeze-body) (error e))").unwrap().display().unwrap(), "(1)", "CODE_LITERAL_RETAINED_IDENTITY");
    let result = loaded.eval_source("(setq freeze-caught nil) (condition-case e (freeze-body) (error (setq freeze-caught t))) (list freeze-caught (+ 40 2))");
    assert!(result.is_ok(), "CODE_LITERAL_MUTATION_CONTINUES: {result:?}");
    assert_eq!(result.unwrap().display().unwrap(), "(nil 42)", "CODE_LITERAL_MUTATION_DOES_NOT_SIGNAL");
}

#[test]
fn cold_image_caches_not_shared_with_live_request_code() {
    let store = Store::new();
    let mut source = Vm::new();
    let install = compile(
        &mut source,
        json!([
            "progn",
            ["defun", "cold", [], ["+", ["cold2"], 22]],
            ["defun", "cold2", [], ["+", 10, 10]]
        ]),
    );
    source.execute(install).unwrap();
    let entry = compile(&mut source, json!(["cold"]));
    let key = source
        .publish_code_image(&store.0, entry, [19; 32])
        .unwrap();
    let (mut vm, entry) = Vm::from_code_image(store.open(&key), [19; 32]).unwrap();
    vm.collect_garbage();
    for i in 0..8 {
        let name = format!("keep{i}");
        vm.eval_form(
            &Form::from_json(json!([
                "setq",
                name,
                [
                    "lambda",
                    [],
                    [
                        "list",
                        ["-", 50, 8],
                        ["length", ["list", 1]],
                        ["car", ["list", i]]
                    ]
                ]
            ]))
            .unwrap(),
        )
        .unwrap();
    }
    vm.collect_garbage();
    for i in 0..8 {
        let v = vm
            .eval_form(&Form::from_json(json!(["car", ["funcall", format!("keep{i}")]])).unwrap())
            .unwrap();
        assert_eq!(v.as_integer().unwrap(), 42);
    }
    let result = vm.execute(entry);
    assert!(result.is_ok(), "cold image cache collision: {result:?}");
    assert_eq!(result.unwrap().as_integer().unwrap(), 42);
    for i in 0..8 {
        let v = vm
            .eval_form(&Form::from_json(json!(["car", ["funcall", format!("keep{i}")]])).unwrap())
            .unwrap();
        assert_eq!(v.as_integer().unwrap(), 42);
    }
}

#[test]
fn runtime_condition_projection_survives_snapshot_without_redefinition() {
    let store = Store::new();
    let mut source = Vm::new();
    let setup = compile(
        &mut source,
        json!(["progn",
        ["define-error", ["quote", "s49-parent"], {"string":"Parent"}],
        ["define-error", ["quote", "s49-child"], {"string":"Child"}, ["quote", "s49-parent"]]]),
    );
    source.execute(setup).unwrap();
    source.collect_garbage();
    let entry = compile(
        &mut source,
        json!([
            "condition-case",
            "e",
            ["signal", ["quote", "s49-child"], ["quote", [41, 42]]],
            ["s49-parent", ["error-message-string", "e"]]
        ]),
    );
    let key = source
        .publish_code_image(&store.0, entry, [50; 32])
        .unwrap();
    let (mut loaded, entry) = Vm::from_code_image(store.open(&key), [50; 32]).unwrap();
    loaded.set_collection_floor(1);
    loaded.collect_garbage();
    let answer = loaded.execute(entry).unwrap();
    assert_eq!(answer.display().unwrap(), "\"Child: 41, 42\"");
}

#[test]
fn protected_regions_roundtrip_with_moving_pending_payloads() {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/control-known-answers.json")).unwrap();
    let store = Store::new();
    assert_eq!(evidence["regime"], "GNU byte-compiled / VM2 compiled");
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 15);
    for row in evidence["cases"].as_array().unwrap() {
        let mut source = Vm::new();
        let entry = compile(
            &mut source,
            json!([
                "progn",
                [
                    "dotimes",
                    ["s49-garbage", 20],
                    ["cons", "s49-garbage", "nil"]
                ],
                row["transport"]
            ]),
        );
        let key = source
            .publish_code_image(&store.0, entry, [49; 32])
            .unwrap();
        let (mut loaded, entry) = Vm::from_code_image(store.open(&key), [49; 32]).unwrap();
        loaded.set_collection_floor(1);
        let value = loaded.execute(entry).unwrap();
        assert_eq!(
            value.display().unwrap(),
            row["expected"].as_str().unwrap(),
            "{}",
            row["name"]
        );
        assert!(loaded.heap_stats().collections > 0);
        assert!(loaded.heap_stats().reclaimed > 0);
    }
}

#[test]
fn uninterned_annotation_specials_remain_dynamic_after_image_restore() {
    let store = Store::new();
    let mut source = Vm::new();
    let entry = compile(&mut source, json!(42));
    let key = source.publish_code_image(&store.0, entry, [5; 32]).unwrap();
    let (mut vm, _) = Vm::from_code_image(store.open(&key), [5; 32]).unwrap();
    let entry = compile(
        &mut vm,
        json!(["let", [["text-property-default-nonsticky", "nil"]],
        ["with-temp-buffer", ["insert", {"string":"ab"}], ["put-text-property",1,3,["quote","display"],7],
            ["goto-char",2], ["insert-and-inherit",{"string":"X"}], ["get-text-property",2,["quote","display"]]]]),
    );
    assert_eq!(vm.execute(entry).unwrap().as_integer().unwrap(), 7);
}

#[test]
fn executable_roundtrip_is_lazy_deterministic_and_preserves_symbols_and_ir() {
    let store = Store::new();
    let mut source = Vm::new();
    let cold = compile(&mut source, json!(["+", 9000, 1]));
    let entry = compile(
        &mut source,
        json!([
            "progn",
            ["defun", "plus-two", ["x"], ["+", "x", 2]],
            ["plus-two", 40]
        ]),
    );
    let ir = source.dump_ir(entry).unwrap();
    let key = source.publish_code_image(&store.0, entry, [3; 32]).unwrap();
    assert_eq!(
        key,
        source.publish_code_image(&store.0, entry, [3; 32]).unwrap()
    );
    let image = store.open(&key);
    let (mut loaded, id) = Vm::from_code_image(image.clone(), [3; 32]).unwrap();
    assert_eq!(id, entry);
    assert_eq!(loaded.image_functions_materialized(), 0);
    assert_eq!(loaded.dump_ir(id).unwrap(), ir);
    assert_eq!(
        loaded.image_functions_materialized(),
        0,
        "read-only IR dump must not install code"
    );
    let answer = loaded.execute(id).unwrap();
    assert_eq!(answer.as_integer().unwrap(), 42);
    assert_eq!(
        loaded.image_functions_materialized(),
        2,
        "unused function must not materialize"
    );
    let follow = compile(&mut loaded, json!(["plus-two", 5]));
    assert_eq!(loaded.execute(follow).unwrap().as_integer().unwrap(), 7);
    assert_eq!(loaded.execute(cold).unwrap().as_integer().unwrap(), 9001);
}

#[test]
fn untouched_executable_corruption_is_rejected_at_first_call_before_answer() {
    let store = Store::new();
    let mut source = Vm::new();
    let cold = compile(&mut source, json!(42));
    let entry = compile(&mut source, json!(7));
    let key = source.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let bytes = std::fs::read(store.0.join(&key)).unwrap();
    // Change a valid constant operand, not JSON syntax, in a page untouched by metadata.
    let start = body_at(&bytes, cold + 1);
    let text = std::str::from_utf8(&bytes[start..]).unwrap();
    let at = start + text.find("169").unwrap(); // tagged fixnum 42
    corrupt(&store.0, &key, at);
    let image = store.open(&key);
    let (mut loaded, id) = Vm::from_code_image(image.clone(), [0; 32]).unwrap();
    assert_eq!(loaded.execute(id).unwrap().as_integer().unwrap(), 7);
    assert_eq!(loaded.image_functions_materialized(), 1);
    let error = loaded
        .execute(cold)
        .expect_err("corrupt executable page must be rejected before an answer");
    assert!(error.contains("hash mismatch"), "{error}");
}

#[test]
fn symbols_in_mapped_lists_keep_identity_across_compaction_and_fresh_loads() {
    let store = Store::new();
    let mut vm = Vm::new();
    let entry = compile(&mut vm, json!(["quote", ["alpha", "beta", "alpha"]]));
    let key = vm.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let image = store.open(&key);
    let (mut a, entry) = Vm::from_code_image(image.clone(), [0; 32]).unwrap();
    let (mut b, _) = Vm::from_code_image(image, [0; 32]).unwrap();
    let list = a.execute(entry).unwrap();
    let alpha = list.cons_pair().unwrap().0;
    let b_list = b.execute(entry).unwrap();
    // Bits, not `Local` equality: these are two different VMs.
    assert_eq!(format!("{alpha:?}"), format!("{:?}", b_list.cons_pair().unwrap().0));
    assert_eq!(list.display().unwrap(), "(alpha beta alpha)");
    let bits = [format!("{list:?}"), format!("{alpha:?}")];
    let (list, alpha) = (list.root(), alpha.root());
    let private = compile(&mut a, json!(["cons", 1, 2]));
    a.execute(private).unwrap();
    a.collect_garbage();
    let after = [format!("{:?}", list.get(&a).unwrap()), format!("{:?}", alpha.get(&a).unwrap())];
    assert_eq!(after, bits);
    assert_eq!(list.get(&a).unwrap().display().unwrap(), "(alpha beta alpha)");
    let quote = compile(&mut a, json!(["quote", "alpha"]));
    let quoted = format!("{:?}", a.execute(quote).unwrap());
    assert_eq!(quoted, bits[1]);
}

#[test]
fn snapshot_rejects_string_properties_before_publishing_any_files() {
    for property in [
        json!("nil"),
        json!(["quote", "bold"]),
        json!(["cons", 17, 23]),
    ] {
        let store = Store::new();
        let mut vm = Vm::new();
        let setup = compile(
            &mut vm,
            json!(["progn",
            ["setq", "s", {"string":"abc"}],
            ["put-text-property", 0, 3, ["quote", "face"], property, "s"]]),
        );
        vm.execute(setup).unwrap();
        let entry = compile(&mut vm, json!(["text-properties-at", 0, "s"]));
        let result = vm.execute(entry).unwrap();
        assert_ne!(result.display().unwrap(), "nil");
        let error = vm
            .publish_code_image(&store.0, entry, [8; 32])
            .expect_err("property-bearing strings must not be silently flattened");
        assert!(error.contains("string text properties"), "{error}");
        assert_eq!(std::fs::read_dir(&store.0).unwrap().count(), 0);
    }
}

#[test]
fn snapshot_restores_definitions_without_reexecuting_the_defining_form() {
    let store = Store::new();
    let mut vm = Vm::new();
    let setup = compile(
        &mut vm,
        json!([
            "progn",
            ["setq", "counter", 0],
            ["defun", "answer", [], ["+", "counter", 42]]
        ]),
    );
    vm.execute(setup).unwrap();
    let entry = compile(&mut vm, json!(["answer"]));
    let key = vm.publish_code_image(&store.0, entry, [8; 32]).unwrap();
    let image = store.open(&key);
    let (mut loaded, id) = Vm::from_code_image(image, [8; 32]).unwrap();
    assert_eq!(loaded.execute(id).unwrap().as_integer().unwrap(), 42);
    assert_eq!(loaded.image_functions_materialized(), 2);
    assert_eq!(loaded.execute(id).unwrap().as_integer().unwrap(), 42);
}

fn reseal(store: &Path, bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut manifest = b"VM2PAGE1".to_vec();
    manifest.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    for page in bytes.chunks(4096) {
        manifest.extend_from_slice(&Sha256::digest(page));
    }
    let key = Sha256::digest(&manifest)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    std::fs::write(store.join(&key), bytes).unwrap();
    std::fs::write(store.join(format!("{key}.pages")), manifest).unwrap();
    key
}

#[test]
fn stale_compiler_and_authenticated_invalid_registers_are_rejected() {
    let store = Store::new();
    let mut vm = Vm::new();
    let entry = compile(&mut vm, json!(42));
    let key = vm.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let original = std::fs::read(store.0.join(&key)).unwrap();
    let mut bytes = original.clone();
    let at = body_at(&bytes, 0)
        + std::str::from_utf8(&bytes[body_at(&bytes, 0)..])
            .unwrap()
            .find("\"compiler\":\"")
            .unwrap()
        + 12;
    bytes[at] = if bytes[at] == b'0' { b'1' } else { b'0' };
    let stale = reseal(&store.0, &bytes);
    let error = Vm::from_code_image(store.open(&stale), [0; 32])
        .err()
        .unwrap();
    assert!(error.contains("compiler/ABI mismatch"), "{error}");
    let mut bytes = original;
    let start = body_at(&bytes, entry + 1);
    let at = start
        + std::str::from_utf8(&bytes[start..])
            .unwrap()
            .find("[15,")
            .unwrap()
        + 4;
    assert!(bytes[at].is_ascii_digit());
    bytes[at] = b'9';
    let invalid = reseal(&store.0, &bytes);
    let (mut loaded, id) = Vm::from_code_image(store.open(&invalid), [0; 32]).unwrap();
    let error = loaded.execute(id).unwrap_err();
    assert!(
        error.contains("invalid executable image operand"),
        "{error}"
    );
}

#[test]
fn hundred_thousand_symbol_slots_keep_identity_without_recompilation() {
    let store = Store::new();
    let mut source = Vm::new();
    let mut names = Vec::new();
    for i in 0..100_000 {
        names.push(Form::Sym(format!("image-symbol-{i}")));
    }
    let entry = source
        .compile(&Form::List(vec![
            Form::Sym("quote".into()),
            Form::List(names),
        ]))
        .unwrap();
    let key = source.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let image = store.open(&key);
    let (mut a, entry) = Vm::from_code_image(image.clone(), [0; 32]).unwrap();
    let (mut b, _) = Vm::from_code_image(image, [0; 32]).unwrap();
    let mut av = a.execute(entry).unwrap();
    let mut bv = b.execute(entry).unwrap();
    let mut count = 0;
    while av != romacs_vm::Fixed::NIL {
        let (asym, tail) = av.cons_pair().unwrap();
        let (bsym, other) = bv.cons_pair().unwrap();
        assert_eq!(format!("{asym:?}"), format!("{bsym:?}"));
        av = tail;
        bv = other;
        count += 1;
    }
    assert_eq!(count, 100_000);
    let before = a.image_functions_materialized();
    a.collect_garbage();
    assert_eq!(a.image_functions_materialized(), before);
}

#[test]
fn mapped_literals_work_in_apply_assq_plists_copy_and_unicode_arrays() {
    for (form, expected) in [
        (json!(["apply", ["quote", "+"], ["quote", [1, 2, 3]]]), "6"),
        (
            json!([
                "car",
                ["assq", ["quote", "a"], ["quote", [["a", 42], ["b", 7]]]]
            ]),
            "a",
        ),
        (
            json!(["plist-get", ["quote", ["a", 42, "b", 7]], ["quote", "b"]]),
            "7",
        ),
        (
            json!(["length", ["copy-sequence", ["quote", ["a", "b"]]]]),
            "2",
        ),
        (
            json!([
                "condition-case",
                "e",
                ["aref", ["quote", ["a"]], 0],
                ["wrong-type-argument", ["car", ["cdr", "e"]]]
            ]),
            "arrayp",
        ),
        (
            json!(["condition-case","e",["setcar",{"string":"text"},1],["wrong-type-argument",["car",["cdr","e"]]]]),
            "consp",
        ),
        (json!(["aref", {"string":"αβγ"}, 1]), "946"),
        (
            json!(["let",[["s",["copy-sequence",{"string":"αβγ"}]]],["aset","s",1,955],["aref","s",1]]),
            "955",
        ),
    ] {
        let store = Store::new();
        let mut source = Vm::new();
        let entry = compile(&mut source, form.clone());
        let key = source.publish_code_image(&store.0, entry, [0; 32]).unwrap();
        let (mut vm, id) = Vm::from_code_image(store.open(&key), [0; 32]).unwrap();
        let v = vm
            .execute(id)
            .unwrap_or_else(|e| panic!("mapped literal operation failed: {form}: {e}"));
        assert_eq!(v.display().unwrap(), expected, "{form}");
    }
}

#[test]
fn mapped_calls_unwind_dynamic_bindings_and_collect_runtime_closure_cells() {
    for (form, expected) in [
        (
            json!([
                "progn",
                [
                    "defun",
                    "sum-rest",
                    ["x", "&rest", "xs"],
                    ["apply", ["quote", "+"], "x", "xs"]
                ],
                ["sum-rest", 1, 2, 3, 4]
            ]),
            10,
        ),
        (
            json!([
                "progn",
                [
                    "defun",
                    "sum-down",
                    ["n"],
                    [
                        "if",
                        ["<", "n", 1],
                        0,
                        ["+", "n", ["sum-down", ["1-", "n"]]]
                    ]
                ],
                ["sum-down", 50]
            ]),
            1275,
        ),
        (
            json!([
                "condition-case",
                "e",
                ["car", 1],
                ["wrong-type-argument", 23]
            ]),
            23,
        ),
        (
            json!([
                "catch",
                ["quote", "done"],
                ["funcall", ["lambda", [], ["throw", ["quote", "done"], 37]]]
            ]),
            37,
        ),
        (
            json!([
                "let",
                [["x", 0]],
                [
                    "let",
                    [["f", ["lambda", [], ["setq", "x", ["1+", "x"]]]]],
                    ["funcall", "f"],
                    ["funcall", "f"],
                    "x"
                ]
            ]),
            2,
        ),
    ] {
        let store = Store::new();
        let mut source = Vm::new();
        let entry = compile(&mut source, form.clone());
        let key = source.publish_code_image(&store.0, entry, [0; 32]).unwrap();
        let (mut vm, id) = Vm::from_code_image(store.open(&key), [0; 32]).unwrap();
        vm.set_collection_floor(1);
        for _ in 0..3 {
            assert_eq!(
                vm.execute(id).unwrap().as_integer().unwrap(),
                expected,
                "{form}"
            );
            vm.collect_garbage();
        }
    }
}

#[test]
fn cold_function_pages_are_not_read_when_restoring_a_large_world() {
    let store = Store::new();
    let mut source = Vm::new();
    let mut setup = vec![json!("progn")];
    for i in 0..1000 {
        setup.push(json!(["defun", format!("cold-{i}"), [], ["+", i, 1]]));
    }
    let init = compile(&mut source, serde_json::Value::Array(setup));
    source.execute(init).unwrap();
    let entry = compile(&mut source, json!(["cold-42"]));
    let key = source.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let image = store.open(&key);
    let (mut vm, id) = Vm::from_code_image(image.clone(), [0; 32]).unwrap();
    let metadata = image.verified_pages();
    assert_eq!(vm.image_functions_materialized(), 0);
    assert_eq!(vm.execute(id).unwrap().as_integer().unwrap(), 43);
    assert_eq!(vm.image_functions_materialized(), 2);
    assert!(
        image.verified_pages() <= metadata + 2,
        "metadata={metadata} after={}",
        image.verified_pages()
    );
    assert!(image.verified_pages() * 4096 < image.mapped_bytes() / 4);
}

#[test]
fn executable_cli_loads_exact_key_after_source_is_removed_and_never_falls_back() {
    let store = Store::new();
    let input = store.0.join("setup.json");
    std::fs::write(
        &input,
        json!({"setup":["defun","answer",[],42],"entry":["answer"]}).to_string(),
    )
    .unwrap();
    let produced = std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm-image"))
        .args([
            "--snapshot-code",
            store.0.to_str().unwrap(),
            input.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        produced.status.success(),
        "{}",
        String::from_utf8_lossy(&produced.stderr)
    );
    let built: serde_json::Value = serde_json::from_slice(&produced.stdout).unwrap();
    std::fs::remove_file(input).unwrap();
    let run = |key: &str, config: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
            .args(["--code-image", store.0.to_str().unwrap(), key, config])
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    };
    let good = run(
        built["key"].as_str().unwrap(),
        built["config"].as_str().unwrap(),
    );
    assert!(
        good.status.success(),
        "{}",
        String::from_utf8_lossy(&good.stderr)
    );
    assert_eq!(good.stdout, b"42\n");
    let stats: serde_json::Value = serde_json::from_slice(&good.stderr).unwrap();
    assert_eq!(stats["mapped_image"], built["key"]);
    assert_eq!(stats["startup_compilations"], 0);
    assert_eq!(stats["startup_source_loads"], 0);
    assert_eq!(stats["materialized_functions"], 2);
    for (key, config) in [
        ("0".repeat(64), built["config"].as_str().unwrap().to_owned()),
        (built["key"].as_str().unwrap().to_owned(), "0".repeat(64)),
    ] {
        let bad = run(&key, &config);
        assert!(!bad.status.success());
        assert!(bad.stdout.is_empty());
    }
}

#[test]
fn lisp_condition_handlers_cannot_swallow_executable_image_corruption() {
    let store = Store::new();
    let mut source = Vm::new();
    let entry = compile(
        &mut source,
        json!([
            "progn",
            ["defun", "damaged", [], 42],
            ["condition-case", "e", ["damaged"], ["error", 99]]
        ]),
    );
    let key = source.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let bytes = std::fs::read(store.0.join(&key)).unwrap();
    let start = body_at(&bytes, 1); // nested defun is compiled before its enclosing entry
    let at = start
        + std::str::from_utf8(&bytes[start..])
            .unwrap()
            .find("169")
            .unwrap();
    corrupt(&store.0, &key, at);
    let (mut vm, id) = Vm::from_code_image(store.open(&key), [0; 32]).unwrap();
    let error = vm
        .execute(id)
        .expect_err("image failure must not become a catchable Lisp error");
    assert!(error.contains("hash mismatch"), "{error}");
}

#[test]
fn source_symbols_macros_and_special_flags_survive_executable_images() {
    let store = Store::new();
    let mut source = Vm::new();
    source
        .eval_source("(defmacro image-macro () 42) (defvar image-special 9)")
        .unwrap();
    let entry = source.compile_source("'λ").unwrap();
    let key = source.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let (mut vm, id) = Vm::from_code_image(store.open(&key), [0; 32]).unwrap();
    let v = vm.execute(id).unwrap();
    assert_eq!(v.display().unwrap(), "λ");
    assert_eq!(
        vm.eval_source("(image-macro)")
            .unwrap()
            .as_integer()
            .unwrap(),
        42
    );
    let v=vm.eval_source("(setq retained (let ((image-special 2)) (lambda () image-special))) (funcall retained)").unwrap();
    assert_eq!(v.as_integer().unwrap(), 9);
}

#[test]
fn reentrant_loader_cannot_launder_fatal_image_errors() {
    let store = Store::new();
    let mut source = Vm::new();
    let entry = source.compile_source("(defun damaged () 42)").unwrap();
    source.execute(entry).unwrap();
    let key = source.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let bytes = std::fs::read(store.0.join(&key)).unwrap();
    let at = body_at(&bytes, 1);
    corrupt(&store.0, &key, at);
    std::fs::write(store.0.join("calls-damaged.el"), "(damaged)").unwrap();
    let (mut vm, _) = Vm::from_code_image(store.open(&key), [0; 32]).unwrap();
    vm.set_load_path(std::slice::from_ref(&store.0)).unwrap();
    let error = vm
        .eval_source("(condition-case e (load \"calls-damaged\") (error 99))")
        .expect_err("nested image failure must remain fatal");
    assert!(error.contains("hash mismatch"), "{error}");
}

#[test]
fn snapshot_preserves_redefined_primitive_function_cells() {
    let store = Store::new();
    let mut source = Vm::new();
    let setup = compile(
        &mut source,
        json!(["defun", "abs", ["x"], ["-", "x", 2]]),
    );
    source.execute(setup).unwrap();
    let entry = compile(&mut source, json!(["abs", 42]));
    let key = source.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let (mut vm, id) = Vm::from_code_image(store.open(&key), [0; 32])
        .expect("snapshot must preserve mutable primitive function cells");
    assert_eq!(vm.execute(id).unwrap().as_integer().unwrap(), 40);
}

#[test]
fn mapped_literal_string_printing_preserves_json_escaping_and_exact_length() {
    let store = Store::new();
    let mut source = Vm::new();
    let entry = source
        .compile(&Form::String("\0\u{1}\n\"\\λ".into()))
        .unwrap();
    let ir = source.dump_ir(entry).unwrap();
    let expected = source.execute(entry).unwrap().display().unwrap();
    let key = source.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let (mut vm, id) = Vm::from_code_image(store.open(&key), [0; 32]).unwrap();
    assert_eq!(
        vm.dump_ir(id).unwrap(),
        ir,
        "image IR must preserve literal escapes exactly"
    );
    let value = vm.execute(id).unwrap();
    assert_eq!(value.display().unwrap(), expected);
    assert_eq!(value.typed_value().unwrap(), json!({"type":"string", "value":"\0\u{1}\n\"\\λ"}));
}

#[test]
fn snapshot_refuses_all_string_property_values_before_writing_files() {
    for property in [
        json!(["quote", "bold"]),
        json!("nil"),
        json!(["cons", 7, "nil"]),
    ] {
        let store = Store::new();
        let mut vm = Vm::new();
        let setup = compile(
            &mut vm,
            json!(["progn", ["setq", "s", {"string":"abc"}], ["put-text-property", 0, 3, ["quote", "face"], property, "s"]]),
        );
        vm.execute(setup).unwrap();
        let entry = compile(
            &mut vm,
            json!(["get-text-property", 0, ["quote", "face"], "s"]),
        );
        let error = vm.publish_code_image(&store.0, entry, [0; 32]).unwrap_err();
        assert!(
            error.contains("cannot publish string text properties"),
            "{error}"
        );
        assert_eq!(std::fs::read_dir(&store.0).unwrap().count(), 0);
    }
}

#[test]
fn snapshot_still_publishes_plain_strings() {
    let store = Store::new();
    let mut vm = Vm::new();
    let entry = compile(&mut vm, json!({"string":"plain λ"}));
    let key = vm.publish_code_image(&store.0, entry, [0; 32]).unwrap();
    let (mut restored, entry) = Vm::from_code_image(store.open(&key), [0; 32]).unwrap();
    let value = restored.execute(entry).unwrap();
    assert_eq!(value.typed_value().unwrap(), json!({"type":"string", "value":"plain λ"}));
}

#[test]
fn configuration_key_changes_and_wrong_config_fail_closed() {
    let store = Store::new();
    let mut vm = Vm::new();
    let entry = compile(&mut vm, json!(42));
    let first = vm.publish_code_image(&store.0, entry, [1; 32]).unwrap();
    let second = vm.publish_code_image(&store.0, entry, [2; 32]).unwrap();
    assert_ne!(first, second);
    let error = Vm::from_code_image(store.open(&first), [2; 32])
        .err()
        .unwrap();
    assert!(error.contains("configuration mismatch"));
}

#[test]
fn snapshot_cli_refuses_every_property_route_and_unreachable_garbage() {
    let routes = [
        json!(["setq", "kept", "s"]),
        json!(["setq", "kept", ["cons", "s", "nil"]]),
        json!(["setq", "kept", ["cons", 1, ["cons", "s", "nil"]]]),
        json!(["put", ["quote", "owner"], ["quote", "p"], "s"]),
        json!(["put", ["quote", "category"], ["quote", "p"], "s"]),
        json!(["kill-new", "s"]),
        json!([
            "setq",
            "default-text-properties",
            ["cons", ["quote", "p"], ["cons", "s", "nil"]]
        ]),
        json!("nil"),
    ];
    let properties = [
        json!("nil"),
        json!(["quote", "bold"]),
        json!(["cons", 7, "nil"]),
        json!({"string":"value"}),
        json!(["quote", "category"]),
    ];
    let mut setups = Vec::new();
    for route in routes {
        for property in &properties {
            setups.push(json!(["let", [["s", ["copy-sequence", {"string":"abc"}]]],
                ["put-text-property", 0, 3, ["quote", "face"], property, "s"], route]));
        }
    }
    setups.push(json!(["progn", ["defun", "literal", [], {"string":"abc"}],
        ["put-text-property", 0, 3, ["quote", "face"], 7, ["literal"]]]));
    for (case, setup) in setups.into_iter().enumerate() {
        let store = Store::new();
        let input = store.0.join("setup.json");
        let output = store.0.join("image");
        std::fs::create_dir(&output).unwrap();
        std::fs::write(&input, json!({"setup":setup,"entry":42}).to_string()).unwrap();
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm-image"))
            .arg("--snapshot-code")
            .arg(&output)
            .arg(&input)
            .output()
            .unwrap();
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(!result.status.success(), "case {case} accepted");
        assert!(
            error.contains("cannot publish string text properties"),
            "case {case}: {error}"
        );
        assert_eq!(
            std::fs::read_dir(&output).unwrap().count(),
            0,
            "case {case}"
        );
    }
}
