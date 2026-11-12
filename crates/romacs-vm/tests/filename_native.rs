use romacs_vm::Vm;
use sha2::{Digest, Sha256};

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source)
        .and_then(|v| v.display())
        .unwrap_or_else(|e| e.to_string())
}

#[test]
fn filename_native_recorded_gnu_cases() {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/filename-native-gnu.json")).unwrap();
    assert_eq!(
        evidence["gnu_commit"],
        "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
    );
    let root = std::env::temp_dir().join(format!("filename-native-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    for name in ["zeta", "alpha", "a.el", ".hidden", "é"] {
        std::fs::write(root.join(name), "").unwrap();
    }
    std::fs::create_dir(root.join("sub")).unwrap();
    let mut mismatches = Vec::new();
    for row in evidence["cases"].as_array().unwrap() {
        let mut vm = Vm::new();
        if row["real_helper"].as_bool() == Some(true) {
            let env = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lisp/gnu/env.el");
            vm.load_file(&env).expect("load real GNU env.el");
        }
        let source = row["source"]
            .as_str()
            .unwrap()
            .replace("@ROOT@", root.to_str().unwrap());
        let actual = eval(&mut vm, &format!("(condition-case e {source} (error e))"))
            .replace(root.to_str().unwrap(), "@ROOT@");
        if let Some(expected) = row["candidate_error"].as_str() {
            if actual != expected {
                mismatches.push(format!(
                    "{}: expected boundary {expected}, got {actual}",
                    row["id"]
                ));
            }
            continue;
        }
        if let Some(unsupported) = row["unsupported"].as_str() {
            assert!(
                actual.contains(&format!("VM2 {unsupported} is unsupported")),
                "unsupported boundary {}: {actual}",
                row["id"]
            );
            continue;
        }
        let expected = row["expected"].as_str().unwrap();
        let expected_form = expected.replace("@ROOT@", root.to_str().unwrap());
        let equal = eval(
            &mut vm,
            &format!("(equal (condition-case e {source} (error e)) '{expected_form})"),
        );
        if equal != "t" {
            mismatches.push(format!("{}: expected {expected}, got {actual}", row["id"]));
        }
    }
    assert!(
        mismatches.is_empty(),
        "filename GNU row mismatch:\n{}",
        mismatches.join("\n")
    );
}

#[test]
fn filename_native_helper_gc() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    assert_eq!(
        eval(
            &mut vm,
            "(defalias 'substitute-env-in-file-name (lambda (s) (let ((i 0)) (while (< i 20000) (cons s s) (setq i (1+ i)))) (concat s \"//result\")))"
        ),
        "substitute-env-in-file-name"
    );
    let before = vm.heap_stats().collections;
    assert_eq!(
        eval(
            &mut vm,
            "(equal (substitute-in-file-name (concat \"before/\" \"/input\")) \"/result\")"
        ),
        "t"
    );
    assert!(
        vm.heap_stats().collections > before,
        "helper must actually collect"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(defalias 'substitute-env-in-file-name (lambda (s) (let ((i 0)) (while (< i 20000) (cons s s) (setq i (1+ i)))) (aset s 0 47) (aset s 1 47) s))"
        ),
        "substitute-env-in-file-name"
    );
    let before = vm.heap_stats().collections;
    assert_eq!(
        eval(
            &mut vm,
            "(let ((s (copy-sequence \"axb\"))) (let ((result (substitute-in-file-name s))) (list (eq s result) (equal result \"//b\"))))"
        ),
        "(t t)"
    );
    assert!(
        vm.heap_stats().collections > before,
        "identity continuation must collect"
    );
}

#[test]
fn filename_native_real_lisp_helper() {
    let mut vm = Vm::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lisp/gnu/env.el");
    assert_eq!(
        Sha256::digest(std::fs::read(&root).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "72daedd2b90f833151a4100f76315d111b4517eb5c75723d47a41a68e6ac105e"
    );
    vm.load_file(&root).expect("load real GNU env.el");
    assert_eq!(
        eval(
            &mut vm,
            "(equal (substitute-in-file-name \"prefix/$$LITERAL\") \"prefix/$LITERAL\")"
        ),
        "t"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(equal (substitute-in-file-name \"before//$$UNDEFINED/~/result\") \"~/result\")"
        ),
        "t"
    );
}
