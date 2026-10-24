use super::*;
fn eval(source: &str) -> String {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let value = vm.eval_source_raw(source).unwrap();
    String::from_utf8(
        vm.str_view(value)
            .unwrap_or_else(|| panic!("not a string: {}", vm.display(value).unwrap()))
            .bytes
            .to_vec(),
    )
    .unwrap()
}
#[test]
fn banked_map_keymap_oracle() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/ert-path-stop/oracle.json")).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        assert_eq!(
            eval(case["source"].as_str().unwrap()),
            case["expected"].as_str().unwrap(),
            "{}",
            case["id"]
        );
    }
}
#[test]
fn map_keymap_live_sparse_vector_parent_and_sort() {
    assert_eq!(
        eval(
            "(prin1-to-string (let* ((v (vector 'a 'b 'c)) (km (list 'keymap v)) (seen nil)) (map-keymap (lambda (k x) (setq seen (cons (cons k x) seen)) (if (= k 0) (aset v 2 'z))) km) (nreverse seen)))"
        ),
        "((0 . a) (1 . b) (2 . z))"
    );
    assert_eq!(
        eval(
            "(prin1-to-string (let* ((km (list 'keymap (cons 'a 'x) (cons 'b 'y))) (seen nil)) (map-keymap (lambda (k x) (setq seen (cons (cons k x) seen)) (if (eq k 'a) (setcdr (cdr km) (list (cons 'c 'z))))) km) (nreverse seen)))"
        ),
        "((a . x) (c . z))"
    );
    assert_eq!(
        eval(
            "(prin1-to-string (let ((seen nil) (parent '(keymap (p . t)))) (map-keymap (lambda (k x) (setq seen (cons (cons k x) seen))) (cons 'keymap (cons '(c . nil) parent))) (nreverse seen)))"
        ),
        "((c) (p))"
    );
    assert_eq!(
        eval(
            "(progn (defun map-keymap-sorted (f m) (list 'called f m)) (prin1-to-string (map-keymap 'f 'm t)))"
        ),
        "(called f m)"
    );
    assert_eq!(
        eval(
            "(prin1-to-string (let ((p '(keymap (p . q)))) (eq p (map-keymap-internal (lambda (k v) nil) (cons 'keymap (cons '(a . b) p))))))"
        ),
        "t"
    );
}
#[test]
fn map_keymap_errors_unwind_and_detached_gc() {
    assert_eq!(
        eval(
            "(prin1-to-string (let ((clean nil)) (list (catch 'done (unwind-protect (map-keymap (lambda (k v) (throw 'done v)) '(keymap (a . b))) (setq clean t))) clean)))"
        ),
        "(b t)"
    );
    assert_eq!(
        eval(
            "(prin1-to-string (condition-case e (map-keymap (lambda (k v) (error \"callback\")) '(keymap (a . b))) (error e)))"
        ),
        "(error \"callback\")"
    );
    assert_eq!(
        eval(
            "(prin1-to-string (let ((ct (make-char-table nil)) (seen nil)) (set-char-table-range ct '(0 . 65535) 'x) (set-char-table-range ct '(65536 . 131071) 'y) (aset ct 65537 (list 'detached)) (map-keymap (lambda (k v) (setq seen (cons (cons k v) seen)) (if (eq v 'x) (progn (set-char-table-range ct '(65536 . 131071) 'replacement) (garbage-collect)))) (list 'keymap ct)) (nreverse seen)))"
        ),
        "(((0 . 65535) . x) (65536 . y) (65537 detached) ((65538 . 131071) . y))"
    );
}
#[test]
fn map_keymap_deep_copy_and_char_table_parent() {
    assert_eq!(
        eval(
            "(prin1-to-string (let* ((ct (make-char-table nil)) (seen nil)) (aset ct 2 'a) (let ((cp (copy-sequence ct))) (aset ct 2 'b) (map-keymap (lambda (k v) (setq seen (cons (cons k v) seen))) (list 'keymap cp))) (nreverse seen)))"
        ),
        "((2 . a))"
    );
    assert_eq!(
        eval(
            "(prin1-to-string (let ((ct (make-char-table nil)) (p (make-char-table nil)) (seen nil)) (aset p 1 'p) (aset ct 3 'c) (set-char-table-parent ct p) (map-keymap (lambda (k v) (setq seen (cons (cons k v) seen))) (list 'keymap ct)) (nreverse seen)))"
        ),
        "((1 . p) (3 . c))"
    );
}

#[test]
fn map_keymap_real_autoload_top_and_embedded() {
    let path = std::path::PathBuf::from(std::env::var("TMPDIR").unwrap())
        .join(format!("map-keymap-autoload-{}.el", std::process::id()));
    std::fs::write(&path, "(fset 'map-autoload '(keymap (loaded . works)))\n").unwrap();
    for map in [
        "'map-autoload",
        "(list 'keymap '(first . before) 'map-autoload)",
    ] {
        let source = format!(
            "(progn (fset 'map-autoload '(autoload \"{}\" nil nil keymap)) (prin1-to-string (let (seen) (map-keymap (lambda (k v) (setq seen (cons (cons k v) seen))) {}) (nreverse seen))))",
            path.display(),
            map
        );
        let expected = if map == "'map-autoload" {
            "((loaded . works))"
        } else {
            "((first . before) (loaded . works))"
        };
        assert_eq!(eval(&source), expected);
        let oracle = std::process::Command::new(std::env::var("ROMACS_GNU_ORACLE").unwrap())
            .args(["--batch", "-Q", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(
            oracle.status.success(),
            "{}",
            String::from_utf8_lossy(&oracle.stderr)
        );
        assert_eq!(String::from_utf8(oracle.stdout).unwrap(), expected);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn map_keymap_default_parent_empty_initial_range() {
    assert_eq!(
        eval(
            "(prin1-to-string (let ((ct (make-char-table nil)) (p (make-char-table nil 'p)) seen) (set-char-table-range ct nil 'c) (set-char-table-parent ct p) (map-keymap (lambda (k v) (setq seen (cons (cons k v) seen))) (list 'keymap ct)) (nreverse seen)))"
        ),
        "(((0 . -1) . p) ((0 . 4194303) . c))"
    );
}

#[test]
fn map_keymap_parent_frame_uses_shared_range_start() {
    assert_eq!(
        eval(
            "(prin1-to-string (let ((ct (make-char-table nil 'a)) (p (make-char-table nil 'r)) out) (set-char-table-range ct nil nil) (set-char-table-range ct '(131200 . 196607) nil) (set-char-table-range ct '(196608 . 4194303) 'b) (set-char-table-range p '(0 . 65535) 'p) (set-char-table-range p '(65536 . 131071) 'q) (set-char-table-parent ct p) (map-keymap (lambda (k v) (setq out (cons (cons k v) out))) (list 'keymap ct)) (nreverse out)))"
        ),
        "(((0 . 131199) . a) ((131200 . 131071) . p) ((131072 . 196607) . r) ((196608 . 4194303) . b))"
    );
}
