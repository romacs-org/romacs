use romacs_vm::Vm;

// generate-new-buffer is GNU Lisp, not a native VM primitive. Load its real
// definition before replaying the inherited marker corpus.
fn prepared_vm() -> Vm {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lisp");
    let mut vm = Vm::new();
    vm.set_load_path(&[root.clone(), root.join("emacs-lisp")]).unwrap();
    vm.install_lisp_prelude().unwrap();
    for name in ["emacs-lisp/debug-early", "emacs-lisp/byte-run", "emacs-lisp/backquote", "subr"] {
        vm.load_file(&root.join(format!("{name}.el"))).unwrap();
    }
    // These fixtures contain no subprocesses; isolate marker lifetime from
    // GNU's unrelated process query hook (the process substrate is absent).
    vm.eval_source("(setq kill-buffer-query-functions nil)").unwrap();
    vm
}

#[test]
fn match_data_after_buffer_death_returns_detached_markers() {
    let mut vm = prepared_vm();
    vm.set_collection_floor(1);
    let actual = vm.eval_source(r#"(let ((b (generate-new-buffer "match-dead"))) (set-buffer b) (insert "abc") (goto-char 1) (looking-at "ab") (kill-buffer b) (condition-case e (let ((m (car (match-data)))) (list (eq (type-of m) 'marker) (marker-position m) (marker-buffer m))) (error e)))"#).unwrap().display().unwrap();
    assert_eq!(actual, "(t nil nil)", "DEAD_BUFFER_MATCH_DATA");
}

#[test]
fn match_data_live_gnu_matrix() {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../tools/primh/evidence/match-data.json")).unwrap();
    use sha2::{Digest, Sha256};
    let hash = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    // match-data uses the legacy search dispatch, not a defsubr declaration.
    // Still bind every recorded input byte and require exact values, no pins.
    for (key, bytes) in [
        (
            "spec_sha256",
            include_bytes!("../tools/primh/frontier5-specs/match-data.json").as_slice(),
        ),
        (
            "generator_sha256",
            include_bytes!("../tools/primh/primh.py").as_slice(),
        ),
        (
            "driver_sha256",
            include_bytes!("../tools/primh/gnu_driver.el").as_slice(),
        ),
        (
            "domains_sha256",
            include_bytes!("../tools/primh/domains.json").as_slice(),
        ),
        (
            "replay_sha256",
            include_bytes!("../tools/primh/replay.rs.in").as_slice(),
        ),
    ] {
        assert_eq!(evidence[key], hash(bytes), "recorded input changed: {key}");
    }
    let rows = evidence["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 22, "complete live-GNU matrix");
    for row in rows {
        let mut vm = prepared_vm();
        vm.set_collection_floor(1);
        let actual = vm
            .eval_source(row["source"].as_str().unwrap())
            .unwrap()
            .display()
            .unwrap();
        let actual: String = serde_json::from_str(&actual).unwrap();
        assert_eq!(
            actual,
            row["expected"].as_str().unwrap(),
            "LIVE_GNU_MATCH_DATA {}",
            row["id"]
        );
    }
}

#[test]
fn loaders_save_match_data_after_buffer_death() {
    let scratch = std::env::var_os("TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".hermes/cache/scratch")
        });
    let dir = scratch.join(format!("match-data-loaders-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("md-load.el"),
        "(string-match \"z\" \"xyz\") (defun md-loaded () 'loaded) (provide 'md-load)",
    )
    .unwrap();
    for (call, expected) in [
        ("(md-loaded)", "(loaded (0 0))"),
        ("(require 'md-load)", "(md-load (0 0))"),
    ] {
        let mut vm = prepared_vm();
        vm.set_collection_floor(1);
        vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
        vm.eval_source("(autoload 'md-loaded \"md-load\")").unwrap();
        let source = format!(
            r#"(let ((b (generate-new-buffer "md-load-buffer")))
          (set-buffer b) (insert "abc") (goto-char 1) (looking-at "ab")
          (kill-buffer b) (condition-case e (list {call} (match-data t)) (error e)))"#
        );
        let actual = vm.eval_source(&source).unwrap().display().unwrap();
        assert_eq!(actual, expected, "DEAD_BUFFER_LOADER {call}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
