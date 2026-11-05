//! `insert-file-contents' and `write-region' (prims/fileio.rs) beyond the
//! recorded corpus (tools/primh/fileio-specs): what needs a real file
//! checked byte for byte from Rust, the VM's own collector, or a refusal
//! that must leave the buffer untouched.
//!
//! Every expected value was taken from the pinned GNU oracle
//! (45b0f7699df, tools/primh/logs/fileio/tests-gnu.el and .out) with
//! `format-decode' and `after-insert-file-set-coding' unbound, as in VM2.
//! Strings holding a newline are compared with `equal' inside Lisp: VM2's
//! printer escapes the newline where GNU prints it raw, a printer difference
//! that is not the primitive's.
use romacs_vm::Vm;
use std::path::PathBuf;

/// The printed value, or the VM's error, so a missing or wrong primitive
/// fails at the assertion that names the GNU answer.
fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

/// A fresh directory per test (tests run in parallel threads).
fn dir(test: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("romacs-fileio-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn lisp(path: &std::path::Path) -> String {
    serde_json::to_string(path.to_str().unwrap()).unwrap()
}

/// ASCII: the bare registry (before mule-conf.el) has no coding system
/// that decodes UTF-8, so text these tests read stays 7-bit; GNU's UTF-8
/// decoding is proven in the loadup regime (real_ert_load.rs).
const ORG: &str = "* Heading\nA body\n";

fn reviewed_child_output(child: std::process::Child, marker: &str) -> std::process::Output {
    child_output_with_timeout(child, marker, std::time::Duration::from_secs(5))
}

fn child_output_with_timeout(mut child: std::process::Child, marker: &str, timeout: std::time::Duration) -> std::process::Output {
    let deadline = std::time::Instant::now() + timeout;
    let mut timed_out = false;
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            timed_out = true;
            child.kill().unwrap();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(!timed_out, "{marker}: watchdog reaped stalled child: {}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{marker}: completed child failed: {}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    output
}

#[test]
fn reviewed_must_report_target_close_failure_before_visit_success() {
    if std::env::var_os("ROMACS_FILEIO_CLOSE_CHILD").is_none() {
        let d = dir("reviewed-close");
        let so = d.join("close.so");
        let target = d.join("target");
        let status = std::process::Command::new("cc")
            .args(["-shared", "-fPIC", "-o"]).arg(&so)
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/evidence/fileio-primitives/close-fault.c"))
            .arg("-ldl").status().unwrap();
        assert!(status.success(), "REVIEWED_CLOSE_SHIM_COMPILES");
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "reviewed_must_report_target_close_failure_before_visit_success", "--nocapture"])
            .env("ROMACS_FILEIO_CLOSE_CHILD", "1")
            .env("ROMACS_FILEIO_CLOSE_TARGET", &target)
            .env("LD_PRELOAD", so)
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
            .spawn().unwrap();
        reviewed_child_output(child, "REVIEWED_CLOSE_CHILD_COMPLETES");
        assert_eq!(std::fs::read(target).unwrap(), b"payload", "REVIEWED_CLOSE_REAL_WRITE");
        return;
    }
    let target = PathBuf::from(std::env::var_os("ROMACS_FILEIO_CLOSE_TARGET").unwrap());
    let mut vm = Vm::new();
    for visit in ["nil", "t", "\"new-visited-name\""] {
        let source = format!("(with-temp-buffer (setq buffer-file-name \"original-name\") (list (condition-case e (write-region \"payload\" nil {} nil {visit}) (error (list (car e) (nth 1 e) (nth 2 e)))) buffer-file-name))", lisp(&target));
        assert_eq!(eval(&mut vm, &source), "((file-error \"Write error\" \"Input/output error\") \"original-name\")", "REVIEWED_CLOSE_WRITE_ERROR_NO_VISIT: {visit}");
    }
}

#[test]
fn reviewed_must_bound_stalled_fifo_write_in_completed_subprocess() {
    if std::env::var_os("ROMACS_FILEIO_STALLED_FIFO_CHILD").is_none() {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "reviewed_must_bound_stalled_fifo_write_in_completed_subprocess", "--nocapture"])
            .env("ROMACS_FILEIO_STALLED_FIFO_CHILD", "1")
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
            .spawn().unwrap();
        reviewed_child_output(child, "REVIEWED_FIFO_CHILD_COMPLETES");
        return;
    }
    use std::os::unix::fs::OpenOptionsExt as _;
    let d = dir("reviewed-stalled-fifo");
    let fifo = d.join("pipe");
    let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let _reader = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&fifo).unwrap();
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(length (make-string 1048576 120))"), "1048576", "REVIEWED_FIFO_PAYLOAD_SUPPORTED");
    let source = format!("(with-temp-buffer (setq buffer-file-name \"original-name\") (list (condition-case e (write-region (make-string 1048576 120) nil {} nil t) (error (list (car e) (nth 1 e)))) buffer-file-name))", lisp(&fifo));
    assert_eq!(eval(&mut vm, &source), "((file-error \"Write error\") \"original-name\")", "REVIEWED_FIFO_STALLED_WRITE_ERROR_NO_VISIT");
}

#[test]
fn reviewed_must_check_text_readonly_before_decode_and_refusals() {
    let d = dir("reviewed-readonly-order");
    let invalid = d.join("invalid");
    let crlf = d.join("crlf");
    let valid = d.join("valid");
    std::fs::write(&invalid, [0xff]).unwrap();
    std::fs::write(&crlf, b"x\r\n").unwrap();
    std::fs::write(&valid, b"x").unwrap();
    let mut vm = Vm::new();
    for (path, binding) in [(&invalid, ""), (&crlf, ""), (&valid, "(coding-system-for-read 'latin-1)"), (&valid, "(before-change-functions '(ignore))")] {
        let source = format!("(with-temp-buffer (insert \"keep\") (put-text-property 1 5 'read-only t) (goto-char 3) (let ({binding}) (list (condition-case e (insert-file-contents {}) (error (car e))) (buffer-string) (point) buffer-file-name)))", lisp(path));
        assert_eq!(eval(&mut vm, &source), "(text-read-only \"keep\" 3 nil)", "REVIEWED_READONLY_BEFORE_DECODE_REFUSAL: {binding} {}", path.display());
    }
}

#[test]
fn reviewed_must_bound_circular_property_aliases() {
    if std::env::var_os("ROMACS_FILEIO_CIRCULAR_CHILD").is_none() {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "reviewed_must_bound_circular_property_aliases", "--nocapture"])
            .env("ROMACS_FILEIO_CIRCULAR_CHILD", "1")
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
            .spawn().unwrap();
        reviewed_child_output(child, "REVIEWED_CIRCULAR_ALIAS_COMPLETES");
        return;
    }
    let d = dir("reviewed-circular-aliases");
    let f = d.join("input");
    std::fs::write(&f, b"X").unwrap();
    let mut vm = Vm::new();
    for aliases in ["(let ((a (list 'garbage))) (setcdr a a) a)", "(let ((a (list 'missing))) (setcdr a a) (list (cons 'read-only a)))"] {
        let source = format!("(with-temp-buffer (insert \"ab\") (goto-char 2) (let ((char-property-alias-alist {aliases})) (insert-file-contents {}) (buffer-string)))", lisp(&f));
        assert_eq!(eval(&mut vm, &source), "\"aXb\"", "REVIEWED_CIRCULAR_ALIAS_RESULT");
    }
}

#[test]
fn reviewed_must_tolerate_malformed_property_aliases() {
    let d = dir("reviewed-malformed-aliases");
    let f = d.join("input");
    std::fs::write(&f, b"X").unwrap();
    let mut vm = Vm::new();
    for aliases in ["'(garbage (read-only fio-alias))", "'((other . 42) (read-only fio-alias))", "'((read-only 42 fio-alias))", "'((read-only fio-alias . 42))"] {
        let source = format!("(with-temp-buffer (insert \"ab\") (put-text-property 1 3 'fio-alias t) (goto-char 2) (let ((char-property-alias-alist {aliases})) (list (condition-case e (insert-file-contents {}) (error (car e))) (buffer-string) (point))))", lisp(&f));
        assert_eq!(eval(&mut vm, &source), "(text-read-only \"ab\" 2)", "REVIEWED_MALFORMED_ALIAS_TOLERANCE: {aliases}");
    }
}

#[test]
fn read_only_insertion_boundaries() {
    let d = dir("readonly-boundary");
    let f = d.join("input");
    std::fs::write(&f, b"X").unwrap();
    let empty = d.join("empty");
    std::fs::write(&empty, b"").unwrap();
    let mut vm = Vm::new();
    for (setup, point, inhibition, expected) in [
        ("(put-text-property 1 3 'read-only 'locked)", 2, "nil", "(text-read-only)"),
        ("(put-text-property 1 3 'read-only 'locked)", 3, "nil", "(text-read-only)"),
        ("(put-text-property 1 3 'read-only \"locked\")", 2, "nil", "(text-read-only \"locked\")"),
        ("(put-text-property 1 3 'read-only 'locked)", 1, "nil", "ok"),
        ("(put-text-property 1 3 'read-only 'locked) (put-text-property 1 3 'front-sticky '(read-only))", 1, "nil", "(text-read-only)"),
        ("(put-text-property 1 3 'read-only 'locked) (put-text-property 1 3 'rear-nonsticky '(read-only))", 3, "nil", "ok"),
        ("(put-text-property 1 3 'read-only 'locked)", 2, "'(locked)", "ok"),
        ("(put-text-property 1 3 'read-only 'locked)", 2, "'(other)", "(text-read-only)"),
        ("(put-text-property 1 3 'read-only 'locked)", 2, "t", "ok"),
        ("(put-text-property 1 3 'inhibit-read-only t) (setq buffer-read-only t)", 2, "nil", "ok"),
        ("(put 'fileio-category 'read-only 'locked) (put-text-property 1 3 'category 'fileio-category)", 2, "nil", "(text-read-only)"),
        ("(put 'fileio-category 'read-only 'locked) (put-text-property 1 3 'category 'fileio-category) (put-text-property 1 3 'front-sticky '(category))", 1, "nil", "(text-read-only)"),
        ("(put 'fileio-category 'read-only 'locked) (put-text-property 1 3 'category 'fileio-category) (put-text-property 1 3 'rear-nonsticky '(category))", 3, "nil", "ok"),
    ] {
        let source = format!("(with-temp-buffer (insert \"ab\") (let ((inhibit-read-only t)) {setup}) (goto-char {point}) (let ((inhibit-read-only {inhibition})) (let ((result (condition-case e (progn (insert-file-contents {}) 'ok) (error e)))) (list result (equal (buffer-string) {}) (point)))))", lisp(&f), if expected == "ok" { match point { 1 => "\"Xab\"", 2 => "\"aXb\"", _ => "\"abX\"" } } else { "\"ab\"" });
        assert_eq!(eval(&mut vm, &source), format!("({expected} t {point})"), "FILEIO4 boundary: {source}");
    }
    // fileio.c:4881: an empty read prepares no change, so no hook runs.
    let source = format!("(with-temp-buffer (let ((before-change-functions '(ignore))) (condition-case e (insert-file-contents {}) (error e))))", lisp(&empty));
    assert_eq!(eval(&mut vm, &source), format!("({} 0)", lisp(&empty)), "FILEIO4 empty before-change");
}

#[test]
fn file_errors_precede_unsupported_buffer_modes_and_confirmation() {
    let d = dir("ordering");
    let missing = lisp(&d.join("absent"));
    let directory = lisp(&d);
    let directory_confirmation = format!("(file-error \"File is a directory\" {directory})");
    let directory_exclusive = format!("(file-already-exists \"File exists\" {directory})");
    let mut vm = Vm::new();
    for (form, expected) in [
        (format!("(insert-file-contents {missing} nil nil nil t)"), "(file-missing \"Opening input file\" \"No such file or directory\")"),
        (format!("(insert-file-contents {directory} nil nil nil t)"), "(file-error \"Read error\" \"Is a directory\")"),
        (format!("(progn (set-buffer-multibyte nil) (insert-file-contents {missing}))"), "(file-missing \"Opening input file\" \"No such file or directory\")"),
        (format!("(write-region \"x\" nil {directory} nil nil nil t)"), directory_confirmation.as_str()),
        (format!("(write-region \"x\" nil {directory} nil nil nil 'excl)"), directory_exclusive.as_str()),
    ] {
        let source = format!("(with-temp-buffer (condition-case e {form} (error (list (car e) (nth 1 e) (nth 2 e)))))");
        assert_eq!(eval(&mut vm, &source), expected, "FILEIO5 error ordering: {source}");
    }
}

#[test]
fn reviewed_readonly_and_ordering_boundaries() {
    let d = dir("review-boundaries");
    let f = d.join("input");
    std::fs::write(&f, b"X").unwrap();
    let link = d.join("link");
    std::os::unix::fs::symlink(&d, &link).unwrap();
    let mut vm = Vm::new();
    for (setup, expected) in [
        ("(put 'fio-cat 'read-only t) (put-text-property 1 3 'category 'fio-cat) (put-text-property 1 3 'read-only nil)", "ok"),
        ("(put-text-property 1 3 'category 42)", "ok"),
        ("(setq char-property-alias-alist '((read-only fio-alias))) (put-text-property 1 3 'fio-alias t)", "(text-read-only)"),
    ] {
        let source = format!("(with-temp-buffer (insert \"ab\") (let ((inhibit-read-only t)) {setup}) (goto-char 2) (condition-case e (progn (insert-file-contents {}) 'ok) (error e)))", lisp(&f));
        assert_eq!(eval(&mut vm, &source), expected, "FILEIO4 reviewed property: {source}");
        assert_eq!(eval(&mut vm, "(setq char-property-alias-alist nil)"), "nil");
    }
    let source = format!("(with-temp-buffer (insert \"abc\") (put-text-property 3 4 'inhibit-read-only t) (narrow-to-region 1 3) (goto-char 3) (setq buffer-read-only t) (condition-case e (progn (insert-file-contents {}) 'ok) (error (car e))))", lisp(&f));
    assert_eq!(eval(&mut vm, &source), "ok", "FILEIO4 inhibition beyond narrowed end");
    let source = format!("(with-temp-buffer (insert \"ab\") (put-text-property 1 3 'read-only t) (goto-char 2) (condition-case e (progn (insert-file-contents {} nil 0 0) 'ok) (error e)))", lisp(&f));
    assert_eq!(eval(&mut vm, &source), "ok", "FILEIO4 zero-length requested range");
    let source = format!("(with-temp-buffer (insert \"ab\") (condition-case e (insert-file-contents {} t nil nil t) (error e)))", lisp(&f));
    assert_eq!(eval(&mut vm, &source), format!("({} 1)", lisp(&f)), "FILEIO5 visiting REPLACE (pinned GNU)");
    let source = format!("(condition-case e (write-region \"x\" nil {} nil nil nil t) (error e))", lisp(&link));
    assert_eq!(eval(&mut vm, &source), "(error \"VM2 write-region MUSTBENEW confirmation is unsupported\")", "FILEIO5 symlink directory confirmation");
}

#[test]
fn write_region_returns_nil_for_all_visit_modes() {
    let d = dir("return-nil");
    let mut vm = Vm::new();
    for visit in ["nil", "t", "'silent", "\"elsewhere\""] {
        let source = format!("(with-temp-buffer (write-region \"x\" nil {} nil {visit}))", lisp(&d.join("out")));
        assert_eq!(eval(&mut vm, &source), "nil", "FILEIO7 return value: {source}");
    }
}

#[test]
fn write_region_fsync_default_and_explicit_nil_use_real_syscall() {
    if std::env::var_os("ROMACS_FILEIO_FSYNC_CHILD").is_none() {
        let d = dir("fsync-shim");
        let so = d.join("fsync.so");
        let status = std::process::Command::new("cc").args(["-shared", "-fPIC", "-o"]).arg(&so)
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/evidence/fileio-primitives/fsync-fault.c")).status().unwrap();
        assert!(status.success(), "compile syscall fault shim");
        for mode in ["eio", "eintr", "einval"] {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "write_region_fsync_default_and_explicit_nil_use_real_syscall", "--nocapture"])
                .env("ROMACS_FILEIO_FSYNC_CHILD", "1").env("ROMACS_FILEIO_FSYNC_FAULT", mode).env("LD_PRELOAD", &so).output().unwrap();
            assert!(out.status.success(), "FILEIO6 syscall child: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        }
        return;
    }
    let d = dir("fsync-child");
    let f = lisp(&d.join("out"));
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, &format!("(condition-case e (write-region \"x\" nil {f}) (error e))")), "nil", "FILEIO6 default must not fsync");
    assert_eq!(eval(&mut vm, "write-region-inhibit-fsync"), "t", "FILEIO6 forwarded default");
    let expected = if std::env::var("ROMACS_FILEIO_FSYNC_FAULT").as_deref() == Ok("einval") { "nil" } else { "(file-error \"Write error\" \"Input/output error\")" };
    assert_eq!(eval(&mut vm, &format!("(let ((write-region-inhibit-fsync nil)) (condition-case e (write-region \"y\" nil {f}) (error (list (car e) (nth 1 e) (nth 2 e)))))")), expected, "FILEIO6 nil must fsync");
}

#[test]
fn write_region_devices_report_actual_write_results() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(write-region \"x\" nil \"/dev/null\")"), "nil", "FILEIO8 null device");
    assert_eq!(eval(&mut vm, "(condition-case e (write-region \"x\" nil \"/dev/full\") (error (list (car e) (nth 1 e) (nth 2 e))))"), "(file-error \"Write error\" \"No space left on device\")", "FILEIO8 full device");
}

#[test]
fn visiting_inserts_decoded_text_and_names_the_buffer() {
    let d = dir("visit");
    let f = d.join("notes.org");
    std::fs::write(&f, ORG).unwrap();
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let f = lisp(&f);
    for (source, gnu) in [
        (format!("(with-temp-buffer (let ((r (insert-file-contents {f} t))) (list (equal (car r) {f}) (cdr r) (equal (buffer-string) \"* Heading\nA body\n\") (point) (equal buffer-file-name {f}) last-coding-system-used deactivate-mark)))"),
         "(t (17) t 1 t undecided-unix t)"),
        (format!("(with-temp-buffer (insert \"AB\") (goto-char 2) (let ((r (insert-file-contents {f}))) (list (cdr r) (equal (buffer-string) \"A* Heading\nA body\nB\") (point) buffer-file-name last-coding-system-used)))"),
         "((17) t 2 nil undecided-unix)"),
        (format!("(with-temp-buffer (let ((r (insert-file-contents {f} nil 2 9))) (list (cdr r) (buffer-string) last-coding-system-used)))"),
         "((7) \"Heading\" undecided)"),
        (format!("(with-temp-buffer (insert \"AB\") (let ((m (copy-marker 2)) (m2 (copy-marker 2 t))) (goto-char 2) (insert-file-contents {f}) (list (marker-position m) (marker-position m2) (point))))"),
         "(2 19 2)"),
        (format!("(with-temp-buffer (insert \"ABCD\") (narrow-to-region 2 3) (goto-char 3) (insert-file-contents {f}) (list (point) (point-min) (point-max) (progn (widen) (equal (buffer-string) \"AB* Heading\nA body\nCD\"))))"),
         "(3 2 20 t)"),
        (format!("(with-temp-buffer (setq buffer-read-only t) (let ((inhibit-read-only t)) (cdr (insert-file-contents {f}))))"), "(17)"),
        // An expanded name comes back as the caller's own string (EQ).
        (format!("(let ((file {f})) (with-temp-buffer (list (eq (car (insert-file-contents file t)) file) (eq buffer-file-name file))))"), "(t t)"),
        (format!("(with-temp-buffer (insert-file-contents {f} nil 1.0 3.0) (buffer-string))"), "\" H\""),
    ] {
        assert_eq!(eval(&mut vm, &source), gnu, "{source}");
    }
}

/// A user's journey in Lisp: visit, edit, save with VISIT t; the file on
/// disk must be exactly the edited text's UTF-8, not merely "written".
#[test]
fn an_edited_visit_saves_back_byte_identically() {
    let d = dir("roundtrip");
    let f = d.join("notes.org");
    std::fs::write(&f, ORG).unwrap();
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let source = format!(
        "(with-temp-buffer (insert-file-contents {} t) (goto-char (point-max)) (insert \"édit\\n\") \
         (list (write-region nil nil buffer-file-name nil t) last-coding-system-used))",
        lisp(&f)
    );
    // Bare VM: no mule-conf, so choose_write_coding_system's nil choice
    // (raw-text-unix in GNU) is the native no-conversion (VM2-FILEIO1);
    // real_ert_load.rs proves GNU's utf-8-unix choice in the loadup regime.
    assert_eq!(eval(&mut vm, &source), "(nil no-conversion)");
    assert_eq!(std::fs::read(&f).unwrap(), "* Heading\nA body\nédit\n".as_bytes());
    // CR survives untouched: a unix-EOL coding never converts EOL on write.
    let crlf = d.join("crlf");
    let source = format!("(with-temp-buffer (insert \"a\\r\\nb\") (write-region nil nil {}))", lisp(&crlf));
    assert_eq!(eval(&mut vm, &source), "nil");
    assert_eq!(std::fs::read(&crlf).unwrap(), b"a\r\nb");
}

#[test]
fn write_region_writes_regions_strings_offsets_and_visits() {
    let d = dir("write");
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let f = |n: &str| d.join(n);
    for (source, path, bytes) in [
        // START nil writes the whole buffer although it is narrowed (fileio.c:5593).
        (format!("(with-temp-buffer (insert \"héllo\\nwörld\\n\") (narrow-to-region 3 5) (write-region nil nil {}) (list (point-min) (point-max)))", lisp(&f("n"))),
         f("n"), "héllo\nwörld\n".as_bytes().to_vec()),
        (format!("(with-temp-buffer (insert \"héllo\\nwörld\\n\") (write-region 5 2 {}))", lisp(&f("r"))), f("r"), "éll".as_bytes().to_vec()),
        (format!("(write-region \"λ\\n\" nil {})", lisp(&f("s"))), f("s"), "λ\n".as_bytes().to_vec()),
        (format!("(progn (write-region \"ab\\n\" nil {0}) (write-region \"cd\\n\" nil {0} t) (write-region \"X\" nil {0} 1))", lisp(&f("a"))),
         f("a"), b"aX\ncd\n".to_vec()),
        (format!("(progn (write-region \"ab\" nil {0}) (write-region \"X\" nil {0} 4))", lisp(&f("p"))), f("p"), b"ab\0\0X".to_vec()),
        // A unibyte string is written as its own octets.
        (format!("(write-region \"\\316\\273\\377\" nil {})", lisp(&f("u"))), f("u"), vec![0xce, 0xbb, 0xff]),
    ] {
        let value = eval(&mut vm, &source);
        assert!(value == "nil" || value == "(3 5)", "{source}: {value}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{source}");
    }
    // A unibyte buffer is written as its octets (raw-text, fileio.c:5385).
    let source = format!("(with-temp-buffer (set-buffer-multibyte nil) (insert \"a\\351\\n\") (list (write-region nil nil {}) last-coding-system-used))", lisp(&f("ub")));
    assert_eq!(eval(&mut vm, &source), "(nil no-conversion)");
    assert_eq!(std::fs::read(f("ub")).unwrap(), b"a\xe9\n");
    // The bare registry has no utf-8: naming it is GNU's coding-system-error,
    // raised before the file is opened.
    let source = format!("(let ((coding-system-for-write 'utf-8)) (condition-case e (write-region \"x\" nil {}) (error e)))", lisp(&f("none")));
    assert_eq!(eval(&mut vm, &source), "(coding-system-error utf-8)");
    assert!(!f("none").exists());
    let (vs, other) = (lisp(&f("vs")), lisp(&f("other")));
    let source = format!("(with-temp-buffer (insert \"x\") (write-region nil nil {vs} nil {other}) (list (equal buffer-file-name {other}) (condition-case e (insert-file-contents {other}) (file-missing 'missing))))");
    assert_eq!(eval(&mut vm, &source), "(t missing)");
    assert_eq!(std::fs::read(f("vs")).unwrap(), b"x");
}

/// choose_write_coding_system's Lisp calls (prims/write_coding.rs): the
/// alist's function entry and `select-safe-coding-system-function', the latter
/// on the widened buffer of a nil START. Each collects; the registers carry
/// the choice across.
#[test]
fn write_region_coding_calls_survive_collection() {
    let d = dir("coding-calls");
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let f = lisp(&d.join("out.gc"));
    let source = format!(r#"(progn
      (defalias 'rt-alist (lambda (args) (garbage-collect) (make-list 64 (length args)) 'undecided))
      (defalias 'rt-select (lambda (from to default &rest _) (garbage-collect) (setq rt-seen (list from to (point-min) (point-max) default)) default))
      (let ((file-coding-system-alist '(("\\.gc\\'" . rt-alist))) (select-safe-coding-system-function 'rt-select))
        (with-temp-buffer (insert "héllo\n") (narrow-to-region 2 3)
          (list (write-region nil nil {f}) last-coding-system-used rt-seen (point-min) (point-max)))))"#);
    assert_eq!(eval(&mut vm, &source), "(nil undecided-unix (1 7 1 7 undecided) 2 3)");
    assert_eq!(std::fs::read(d.join("out.gc")).unwrap(), "h\u{e9}llo\n".as_bytes());
}

#[test]
fn file_errors_carry_gnu_conditions_and_data() {
    let d = dir("errors");
    let mut vm = Vm::new();
    let nope = lisp(&d.join("nope"));
    let e = lisp(&d.join("e"));
    let nodir = lisp(&d.join("no/such/x"));
    let dirname = lisp(&d);
    for (source, gnu) in [
        (format!("(condition-case e (insert-file-contents {nope}) (error (list (car e) (nth 1 e) (nth 2 e) (equal (nth 3 e) {nope}))))"),
         "(file-missing \"Opening input file\" \"No such file or directory\" t)"),
        // Visiting completes before the error (fileio.c:5016, 5308).
        (format!("(with-temp-buffer (list (condition-case e (insert-file-contents {nope} t) (error (car e))) (equal buffer-file-name {nope}) (buffer-string) last-coding-system-used))"),
         "(file-missing t \"\" undecided)"),
        (format!("(condition-case e (write-region \"x\" nil {nodir}) (error (list (car e) (nth 1 e) (nth 2 e))))"),
         "(file-missing \"Opening output file\" \"No such file or directory\")"),
        (format!("(progn (write-region \"x\" nil {e}) (condition-case e (write-region \"y\" nil {e} nil nil nil 'excl) (error (list (car e) (nth 1 e) (equal (nth 2 e) {e})))))"),
         "(file-already-exists \"File exists\" t)"),
        (format!("(condition-case e (insert-file-contents {dirname}) (error (list (car e) (nth 1 e) (nth 2 e))))"),
         "(file-error \"Read error\" \"Is a directory\")"),
        (format!("(with-temp-buffer (insert \"abc\") (condition-case e (write-region 10 2 {e}) (error (list (car e) (nth 2 e) (nth 3 e)))))"), "(args-out-of-range 10 2)"),
        (format!("(with-temp-buffer (setq buffer-read-only t) (condition-case e (insert-file-contents {e}) (error (list (car e) (eq (nth 1 e) (current-buffer))))))"), "(buffer-read-only t)"),
        (format!("(condition-case e (insert-file-contents {e} nil -1) (error e))"), "(wrong-type-argument file-offset -1)"),
        (format!("(with-temp-buffer (condition-case e (insert-file-contents {e} t 1) (error e)))"), "(error \"Attempt to visit less than an entire file\")"),
        (format!("(with-temp-buffer (insert \"q\") (condition-case e (insert-file-contents {e} t) (error e)))"), "(error \"Cannot do file visiting in a non-empty buffer\")"),
        (format!("(condition-case e (write-region \"x\" nil {e} nil nil 'foo) (error e))"), "(wrong-type-argument stringp foo)"),
    ] {
        assert_eq!(eval(&mut vm, &source), gnu, "{source}");
    }
}

/// Every unsupported decoding path signals BEFORE the buffer changes.
#[test]
fn unsupported_reads_refuse_and_leave_the_buffer_alone() {
    let d = dir("refuse");
    let crlf = d.join("crlf");
    std::fs::write(&crlf, b"a\r\nb\r\n").unwrap();
    let latin = d.join("latin");
    std::fs::write(&latin, b"caf\xe9\n").unwrap();
    let nul = d.join("nul");
    std::fs::write(&nul, b"a\0b").unwrap();
    let bom = d.join("bom");
    std::fs::write(&bom, b"\xef\xbb\xbfx").unwrap();
    let mut vm = Vm::new();
    let (crlf, latin, nul, bom) = (lisp(&crlf), lisp(&latin), lisp(&nul), lisp(&bom));
    // Bootstrap regime: the bare registry gives no coding category a
    // system, so detect_coding rejects every category in 8-bit text and
    // decides raw-text, which GNU's C does not define before mule-conf.el
    // either (syms_of_coding defines no-conversion and undecided only):
    // setup_coding_system signals before the buffer changes.
    for form in [format!("(insert-file-contents {latin})"), format!("(insert-file-contents {bom})"),
                 // Anchor the nonmatching basename: random TMPDIR components may contain x.
                 format!("(let ((file-name-handler-alist '((\"/handler-only\\\\'\" . ignore)))) (insert-file-contents {latin}))")] {
        let source = format!("(with-temp-buffer (insert \"keep\") (goto-char 3) (list (condition-case e {form} (error e)) (buffer-string) (point) last-coding-system-used))");
        assert_eq!(eval(&mut vm, &source), "((coding-system-error raw-text) \"keep\" 3 nil)", "{form}");
    }
    // An expansion handler that returns no name is invalid (pinned GNU),
    // before the buffer or the file is touched.
    let source = format!("(with-temp-buffer (insert \"keep\") (goto-char 3) (list (condition-case e (let ((file-name-handler-alist '((\"/crlf\\\\'\" . list)))) (insert-file-contents {crlf})) (error e)) (buffer-string) (point) last-coding-system-used))");
    assert_eq!(eval(&mut vm, &source), "((error \"Invalid handler in ‘file-name-handler-alist’\") \"keep\" 3 nil)");
    for (form, message) in [
        (format!("(let ((selective-display t)) (write-region nil nil {crlf}))"), "write-region with selective-display t"),
        (format!("(let ((write-region-annotate-functions '(ignore))) (write-region \"x\" nil {crlf}))"), "write-region annotations"),
        (format!("(write-region \"x\" nil {crlf} nil nil nil t)"), "write-region MUSTBENEW confirmation"),
    ] {
        let source = format!("(with-temp-buffer (insert \"keep\") (goto-char 3) (list (condition-case e {form} (error e)) (buffer-string) (point) last-coding-system-used))");
        assert_eq!(eval(&mut vm, &source), format!("((error \"VM2 {message} is unsupported\") \"keep\" 3 nil)"), "{form}");
    }
    // REPLACE of the whole text, point spread over the new text (pinned GNU).
    let source = format!("(with-temp-buffer (insert \"keep\") (goto-char 3) (list (insert-file-contents {crlf} nil nil nil t) (buffer-string) (point) last-coding-system-used))");
    assert_eq!(eval(&mut vm, &source), format!("(({crlf} 4) \"a\\nb\\n\" 3 undecided-dos)"));
    // Refused writes did not touch the file.
    assert_eq!(std::fs::read(d.join("crlf")).unwrap(), b"a\r\nb\r\n");
    // 7-bit text decodes in the bare regime as in GNU: EOL detection, and a
    // NUL byte selects no-conversion.
    let source = format!("(with-temp-buffer (list (cdr (insert-file-contents {crlf})) (equal (buffer-string) \"a\nb\n\") last-coding-system-used))");
    assert_eq!(eval(&mut vm, &source), "((4) t undecided-dos)");
    let source = format!("(with-temp-buffer (list (cdr (insert-file-contents {nul})) (equal (buffer-string) \"a\\0b\") last-coding-system-used))");
    assert_eq!(eval(&mut vm, &source), "((3) t no-conversion)");
    let source = format!("(with-temp-buffer (let ((coding-system-for-read 'no-conversion)) (list (cdr (insert-file-contents {crlf})) (equal (buffer-string) \"a\\r\nb\\r\n\") last-coding-system-used)))");
    assert_eq!(eval(&mut vm, &source), "((6) t no-conversion)");
}

/// `buffer-file-name' is the one Lisp object a visit leaves behind, held in
/// the buffer's per-buffer slot. It must move with a compacting collection
/// and still read back as the same live string, with the text unchanged.
#[test]
fn the_visited_file_name_survives_a_moving_collection() {
    let d = dir("gc");
    let f = d.join("notes.org");
    std::fs::write(&f, ORG).unwrap();
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    vm.set_collection_floor(1_000_000);
    // Garbage BELOW the visit, so compaction must move what the visit made.
    eval(&mut vm, "(let ((i 0)) (while (< i 2000) (copy-sequence \"garbage\") (setq i (1+ i))))");
    let visit = format!("(progn (setq romacs-gc-b (get-buffer-create (generate-new-buffer-name \"visited\"))) (with-current-buffer romacs-gc-b (insert-file-contents {} t)) nil)", lisp(&f));
    assert_eq!(eval(&mut vm, &visit), "nil");
    let before = vm.eval_source("(with-current-buffer romacs-gc-b buffer-file-name)").unwrap();
    let bits = format!("{before:?}");
    let rooted = before.root();
    vm.collect_garbage();
    let after = rooted.get(&mut vm).unwrap();
    assert_ne!(format!("{after:?}"), bits, "the collection did not move the name; the test proves nothing");
    let live = eval(&mut vm, "(with-current-buffer romacs-gc-b (list buffer-file-name (equal (buffer-string) \"* Heading\nA body\n\")))");
    assert_eq!(live, format!("({} t)", lisp(&f)));
    let same = eval(&mut vm, "(with-current-buffer romacs-gc-b (eq buffer-file-name (with-current-buffer romacs-gc-b buffer-file-name)))");
    assert_eq!(same, "t");
}

/// FIFO writes with a reader deliver bytes; input remains fail-closed and a
/// write with no reader returns ENXIO rather than blocking indefinitely.
#[test]
fn fifo_writes_deliver_to_readers_and_fail_closed_without_one() {
    // A concurrent test can fork while the reader is open. CLOEXEC closes its
    // inherited copy only at exec, so dropping our copy does not yet establish
    // the no-reader precondition. Open the FIFO in an isolated test process.
    if std::env::var_os("ROMACS_FILEIO_FIFO_CHILD").is_none() {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "fifo_writes_deliver_to_readers_and_fail_closed_without_one", "--nocapture"])
            .env("ROMACS_FILEIO_FIFO_CHILD", "1")
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
            .spawn().unwrap();
        child_output_with_timeout(child, "FILEIO8_FIFO_CHILD_COMPLETES", std::time::Duration::from_secs(30));
        return;
    }
    use std::io::Read as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let d = dir("fifo");
    let fifo = d.join("pipe");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0, "mkfifo");
    // A reader, so a write would be delivered if it were attempted.
    let mut reader = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&fifo).unwrap();
    let f = lisp(&fifo);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut vm = Vm::new();
        let write = eval(&mut vm, &format!("(condition-case e (write-region \"leak\" nil {f}) (error e))"));
        let read = eval(&mut vm, &format!("(with-temp-buffer (insert \"keep\") (list (condition-case e (insert-file-contents {f}) (error e)) (buffer-string)))"));
        tx.send((write, read)).unwrap();
    });
    let (write, read) = rx.recv_timeout(std::time::Duration::from_secs(30)).expect("a FIFO blocked the primitive");
    assert_eq!(write, "nil", "FILEIO8 FIFO write result");
    let mut got = vec![];
    match reader.read_to_end(&mut got) {
        Ok(_) => {}
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock, "{e}"),
    }
    assert_eq!(got, b"leak", "FILEIO8 FIFO delivered bytes");
    drop(reader);
    let mut vm = Vm::new();
    let no_reader = format!("(condition-case e (write-region \"x\" nil {}) (error (list (car e) (nth 1 e) (nth 2 e))))", lisp(&fifo));
    assert_eq!(eval(&mut vm, &no_reader), "(file-error \"Opening output file\" \"No such device or address\")", "FILEIO8 no-reader fail-closed");
    assert_eq!(read, "((error \"VM2 insert-file-contents of a non-regular file is unsupported\") \"keep\")");
}
#[test]
fn visited_modtime_records_metadata_not_contents() {
    use std::os::unix::fs::MetadataExt;
    let d = dir("mtime"); let path = d.join("file");
    std::fs::write(&path, "first").unwrap();
    let mut vm = Vm::new();
    eval(&mut vm, "(setq current-time-list t)");
    assert_eq!(eval(&mut vm, "(visited-file-modtime)"), "0", "MTIME_UNKNOWN");
    eval(&mut vm, &format!("(insert-file-contents {} t)", lisp(&path)));
    assert_eq!(eval(&mut vm, "(verify-visited-file-modtime)"), "t", "MTIME_VISIT_RECORDED");
    assert_eq!(eval(&mut vm, "(setq mtime-buffer (current-buffer)) (with-temp-buffer (list (visited-file-modtime) (verify-visited-file-modtime mtime-buffer) (eq mtime-buffer (current-buffer))))"), "(0 t nil)", "MTIME_BUFFER_LOCAL_OPTIONAL_BUFFER");
    let meta = std::fs::metadata(&path).unwrap();
    let seconds = meta.mtime(); let nanos = meta.mtime_nsec();
    let expected = format!("({} {} {} {})", seconds.div_euclid(65536), seconds.rem_euclid(65536), nanos / 1000, nanos % 1000 * 1000);
    assert_eq!(eval(&mut vm, "(visited-file-modtime)"), expected, "MTIME_DESCRIPTOR_TIME_RECORDED");
    let changed = std::time::UNIX_EPOCH + std::time::Duration::new(meta.mtime() as u64 + 2, meta.mtime_nsec() as u32);
    std::fs::File::open(&path).unwrap().set_times(std::fs::FileTimes::new().set_modified(changed)).unwrap();
    assert_eq!(eval(&mut vm, "(verify-visited-file-modtime)"), "nil", "MTIME_TIMESTAMP_ONLY_CHANGE");
    std::fs::write(&path, "other").unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let t = std::time::UNIX_EPOCH + std::time::Duration::new(meta.mtime() as u64, meta.mtime_nsec() as u32);
    file.set_times(std::fs::FileTimes::new().set_modified(t)).unwrap();
    assert_eq!(eval(&mut vm, "(verify-visited-file-modtime)"), "t", "MTIME_NOT_CONTENTS");
    std::fs::write(&path, "longer contents").unwrap(); file.set_times(std::fs::FileTimes::new().set_modified(t)).unwrap();
    assert_eq!(eval(&mut vm, "(verify-visited-file-modtime)"), "nil", "MTIME_SIZE_GUARD");
    assert_eq!(eval(&mut vm, "(set-visited-file-modtime (visited-file-modtime)) (verify-visited-file-modtime)"), "t", "MTIME_EXPLICIT_IGNORES_SIZE");
    eval(&mut vm, "(set-visited-file-modtime)");
    let stamp = eval(&mut vm, "(visited-file-modtime)");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(eval(&mut vm, "(verify-visited-file-modtime)"), "nil", "MTIME_REMOVED");
    assert_eq!(eval(&mut vm, "(set-visited-file-modtime) (visited-file-modtime)"), stamp, "MTIME_MISSING_SETTER_KEEPS_STATE");
    assert_eq!(eval(&mut vm, "(set-visited-file-modtime -1) (list (visited-file-modtime) (verify-visited-file-modtime))"), "(-1 t)", "MTIME_MISSING_FLAG");
    assert_eq!(eval(&mut vm, "(set-visited-file-modtime 0) (list (visited-file-modtime) (verify-visited-file-modtime))"), "(0 t)", "MTIME_UNKNOWN_FLAG");
    assert_eq!(eval(&mut vm, &format!("(write-region \"saved\" nil {} nil t) (verify-visited-file-modtime)", lisp(&path))), "t", "MTIME_WRITE_RECORDED");
    assert_eq!(eval(&mut vm, "(equal (visited-file-modtime) 0)"), "nil", "MTIME_WRITE_NOT_UNKNOWN");
}

#[test]
fn visited_modtime_timeforms_gnu() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(set-visited-file-modtime '(1 2 3 4000)) (visited-file-modtime)"), "(65538000003004 . 1000000000)", "MTIME_DEFAULT_TICKS");
    for (time, expected) in [
        ("'(1 2 . 3)", "(65538000003000 . 1000000000)"),
        ("'(1 2 3 4000 ignored . tail)", "(65538000003004 . 1000000000)"),
        ("'(1 2 3 . ignored)", "(65538000003000 . 1000000000)"),
        ("0.3", "(299999999 . 1000000000)"),
        ("-0.1", "(-100000001 . 1000000000)"),
        ("5.0e-324", "(0 . 1000000000)"),
        ("-5.0e-324", "(-1 . 1000000000)"),
    ] {
        assert_eq!(eval(&mut vm, &format!("(set-visited-file-modtime {time}) (visited-file-modtime)")), expected, "MTIME_GNU_TIMEFORM {time}");
    }
    assert_eq!(eval(&mut vm, "(list (boundp 'current-time-list) current-time-list)"), "(t nil)", "MTIME_FORMAT_VARIABLE");
    assert_eq!(eval(&mut vm, "(let ((current-time-list t)) (visited-file-modtime))"), "(-1 65535 999999 999000)", "MTIME_DYNAMIC_FORMAT_SETTING");
    assert_eq!(eval(&mut vm, "(setq current-time-list t) (visited-file-modtime)"), "(-1 65535 999999 999000)", "MTIME_LIST_SETTING");
    assert_eq!(eval(&mut vm, "(setq current-time-list nil) (visited-file-modtime)"), "(-1 . 1000000000)", "MTIME_PAIR_SETTING");
}

#[test]
fn visited_modtime_default_descriptor_ticks() {
    use std::os::unix::fs::MetadataExt;
    let d = dir("mtime-ticks"); let path = d.join("file");
    std::fs::write(&path, "timestamp").unwrap();
    let mut vm = Vm::new();
    eval(&mut vm, &format!("(insert-file-contents {} t)", lisp(&path)));
    let meta = std::fs::metadata(&path).unwrap();
    let ticks = meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128;
    assert_eq!(eval(&mut vm, "(visited-file-modtime)"), format!("({ticks} . 1000000000)"), "MTIME_DESCRIPTOR_TICKS");
    assert_eq!(eval(&mut vm, "(set-visited-file-modtime (visited-file-modtime)) (verify-visited-file-modtime)"), "t", "MTIME_TICKS_ROUNDTRIP");
}

#[test]
fn visited_modtime_time_errors_preserve_state() {
    let mut vm = Vm::new();
    eval(&mut vm, "(set-visited-file-modtime '(0 1))");
    for (time, message) in [
        ("0.0e+NaN", "Invalid time specification"),
        ("1.0e+INF", "Specified time is not representable"),
        ("-1.0e+INF", "Specified time is not representable"),
        ("9223372036854775808.0", "Specified time is not representable"),
        ("'(140737488355328 0)", "Specified time is not representable"),
        ("'(0 . 0)", "Invalid time specification"),
        ("'(0 1 bogus)", "Invalid time specification"),
        ("'(0 1 0 bogus ignored)", "Invalid time specification"),
    ] {
        assert_eq!(eval(&mut vm, &format!("(condition-case e (set-visited-file-modtime {time}) (error (car (cdr e))))")), format!("\"{message}\""), "MTIME_ERROR {time}");
        assert_eq!(eval(&mut vm, "(visited-file-modtime)"), "(1000000000 . 1000000000)", "MTIME_ERROR_PRESERVES_STATE {time}");
    }
    assert_eq!(eval(&mut vm, "(set-visited-file-modtime '(40000 0)) (condition-case e (visited-file-modtime) (error (car (cdr e))))"), "\"VM2 visited-file-modtime requiring bignum ticks is unsupported\"", "MTIME_BIGNUM_SUBSTRATE_REFUSAL");
    assert_eq!(eval(&mut vm, "(let ((current-time-list t)) (visited-file-modtime))"), "(40000 0 0 0)", "MTIME_BIGNUM_REFUSAL_KEEPS_NATIVE_TIME");
}

#[test]
fn visited_modtime_flags_and_timestamp_forms() {
    let mut vm = Vm::new();
    eval(&mut vm, "(setq current-time-list t)");
    for (time, expected) in [("'(1 2)", "(1 2 0 0)"), ("'(1 2 3 4000)", "(1 2 3 4000)"),
        ("'(3 . 2)", "(0 1 500000 0)"), ("'(-1 65535 999999 999000)", "(-1 65535 999999 999000)"),
        ("1.5", "(0 1 500000 0)")] {
        assert_eq!(eval(&mut vm, &format!("(set-visited-file-modtime {time}) (visited-file-modtime)")), expected, "MTIME_TIME_FORM {time}");
    }
    assert_eq!(eval(&mut vm, "(condition-case e (set-visited-file-modtime 1) (error (car e)))"), "args-out-of-range", "MTIME_FLAG_RANGE");
    let d = dir("mtime-missing"); let path = d.join("absent");
    assert_eq!(eval(&mut vm, &format!("(with-temp-buffer (condition-case nil (insert-file-contents {} t) (file-missing nil)) (list (visited-file-modtime) (verify-visited-file-modtime)))", lisp(&path))), "(-1 t)", "MTIME_MISSING_VISIT");
}

/// fileio.c:5859: an interactive write names what it wrote in the echo
/// area and *Messages*: "Wrote" (a VISIT string's file), "Added to" for
/// an append, "Updated" at an offset, nothing for a quiet VISIT. Pinned
/// GNU under `emacs -nw -Q' answered these; in batch it says nothing.
#[test]
fn write_region_reports_the_file_it_wrote_as_gnu_does() {
    let d = dir("wrote");
    let (a, b) = (lisp(&d.join("a.txt")), lisp(&d.join("b.txt")));
    let mut vm = Vm::new();
    let report = eval(&mut vm, &format!("(progn (setq noninteractive nil)
        (let (out)
          (with-temp-buffer
            (insert \"x\")
            (write-region nil nil {a} nil t) (push (current-message) out) (message nil)
            (write-region nil nil {a} t) (push (current-message) out) (message nil)
            (write-region nil nil {a} 0) (push (current-message) out) (message nil)
            (write-region \"y\" nil {a} nil 'quiet) (push (current-message) out) (message nil)
            (write-region \"y\" nil {a} nil {b}) (push (current-message) out) (message nil)
            (write-region \"y\" nil {a}) (push (current-message) out))
          (push (with-current-buffer \"*Messages*\" (buffer-string)) out)
          (equal (nreverse out)
                 (list (concat \"Wrote \" {a}) (concat \"Added to \" {a}) (concat \"Updated \" {a}) nil
                       (concat \"Wrote \" {b}) (concat \"Wrote \" {a})
                       (concat \"Wrote \" {a} \"\\nAdded to \" {a} \"\\nUpdated \" {a} \"\\nWrote \" {b} \"\\nWrote \" {a} \"\\n\")))))"));
    assert_eq!(report, "t");
    let mut batch = Vm::new();
    let quiet = eval(&mut batch, &format!("(progn (setq noninteractive t)
        (with-temp-buffer (insert \"x\") (write-region nil nil {a} nil t))
        (list (current-message) (get-buffer \"*Messages*\")))"));
    assert_eq!(quiet, "(nil nil)");
}
