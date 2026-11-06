use romacs_vm::Vm;

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/pinned_gnu.rs"));

fn review_symlink_nul_case(collapse: bool) {
    let fixture = Fixture::new();
    std::os::unix::fs::symlink("missing-target", fixture.0.join("valid-symlink")).unwrap();
    let suffix = if collapse { "/../valid-symlink" } else { "" };
    let source = format!(
        "(let ((original (concat {} (string 0) {}))) (condition-case e (file-symlink-p original) (error (list (car e) (nth 1 e) (equal (nth 2 e) original)))))",
        serde_json::to_string(&fixture.0.join("nul")).unwrap(),
        serde_json::to_string(suffix).unwrap()
    );
    let output = std::process::Command::new(pinned_gnu())
        .args(["--batch", "-Q", "--eval", &format!("(princ {source})")])
        .output().expect("pinned GNU is required");
    assert!(output.status.success(), "GNU oracle failed: {output:?}");
    let expected = String::from_utf8(output.stdout).unwrap();
    println!("GNU_EXPECTED_SYMLINK_NUL collapse={collapse}: {expected}");
    assert_eq!(expected, "(wrong-type-argument filenamep t)", "GNU must preserve the original NUL-containing argument");
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, &source), expected, "NUL_REJECTED_BEFORE_PATH_NORMALIZATION collapse={collapse}");
    assert_eq!(std::fs::read_link(fixture.0.join("valid-symlink")).unwrap(), std::path::PathBuf::from("missing-target"));
}

#[test]
fn review_symlink_nul_original_filename_is_rejected() {
    review_symlink_nul_case(false);
}

#[test]
fn review_symlink_nul_parent_component_cannot_reach_valid_link() {
    review_symlink_nul_case(true);
}
fn eval(vm: &mut Vm, text: &str) -> String {
    vm.eval_source(text)
        .map(|v| v.display().unwrap())
        .unwrap_or_else(|e| format!("ERROR {e}"))
}
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let base = std::env::var_os("TMPDIR").expect("TMPDIR must name the scratch directory");
        let path = std::path::PathBuf::from(base).join(format!(
            "savejourney-case-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let probe = path.join("probe");
        std::fs::create_dir(&probe).unwrap();
        Self(probe)
    }
    fn call(&self, vm: &mut Vm, suffix: &str) -> String {
        eval(
            vm,
            &format!(
                "(condition-case e (file-name-case-insensitive-p {}) (error e))",
                serde_json::to_string(&self.0.join(suffix)).unwrap()
            ),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.0.parent().unwrap()).unwrap();
    }
}
/// `set-auto-coding' passes ENCODE_FILE's unibyte name to
/// `auto-coding-alist-lookup', which asks about it: the encoded name answers
/// as its multibyte spelling; bytes that are not UTF-8 are raw-byte
/// characters of the name (DECODE_FILE).
#[test]
fn filename_case_encoded_unibyte_name() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("λ"), b"unchanged").unwrap();
    let mut vm = Vm::new();
    let path = fixture.0.join("λ");
    let name = serde_json::to_string(&path).unwrap();
    let bytes: Vec<String> = path.to_str().unwrap().bytes().map(|b| b.to_string()).collect();
    assert_eq!(
        eval(&mut vm, &format!("(list (file-name-case-insensitive-p {name}) (file-name-case-insensitive-p (unibyte-string {})))", bytes.join(" "))),
        "(nil nil)"
    );
    // Pinned GNU: nil; the byte names a (missing) file with that byte.
    assert_eq!(
        eval(&mut vm, "(condition-case e (file-name-case-insensitive-p (unibyte-string 47 255)) (error e))"),
        "nil"
    );
}
#[test]
fn filename_case_file_directory_missing_and_type_contract() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("a"), b"unchanged").unwrap();
    assert!(
        !fixture.0.join("A").exists(),
        "fixture requires a case-sensitive directory"
    );
    let before: Vec<_> = std::fs::read_dir(&fixture.0)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let mut vm = Vm::new();
    for name in ["a", "", "missing", "missing/nested"] {
        assert_eq!(
            fixture.call(&mut vm, name),
            "nil",
            "CASE_SENSITIVE {name}"
        );
    }
    assert_eq!(
        eval(
            &mut vm,
            &format!(
                "(funcall 'file-name-case-insensitive-p {})",
                serde_json::to_string(&fixture.0.join("a")).unwrap()
            )
        ),
        "nil"
    );
    assert_eq!(std::fs::read(fixture.0.join("a")).unwrap(), b"unchanged");
    let after: Vec<_> = std::fs::read_dir(&fixture.0)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(before, after, "READ_ONLY_NO_PROBE_FILES");
    assert_eq!(
        eval(
            &mut vm,
            "(condition-case e (file-name-case-insensitive-p 2) (error e))"
        ),
        "(wrong-type-argument stringp 2)"
    );
}
#[test]
// An empty directory, or one whose names have no case, answers as any other.
fn filename_case_caseless_directory_answers_as_any_other() {
    let fixture = Fixture::new();
    let mut vm = Vm::new();
    for name in ["", "missing"] {
        assert_eq!(fixture.call(&mut vm, name), "nil", "EMPTY_DIRECTORY");
    }
    std::fs::write(fixture.0.join("123"), b"unchanged").unwrap();
    assert_eq!(fixture.call(&mut vm, "123"), "nil", "NO_CASEABLE_ENTRY");
}
#[test]
fn filename_case_real_alternate_hardlink_is_not_case_folding() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("a"), b"unchanged").unwrap();
    std::fs::hard_link(fixture.0.join("a"), fixture.0.join("A")).unwrap();
    let mut vm = Vm::new();
    // A hardlinked pair that differs only in case is not case folding.
    assert_eq!(fixture.call(&mut vm, "a"), "nil", "REAL_HARDLINK_IS_NOT_FOLDING");
    std::fs::remove_file(fixture.0.join("A")).unwrap();
    std::fs::write(fixture.0.join("A"), b"different inode").unwrap();
    assert_eq!(
        fixture.call(&mut vm, "a"),
        "nil",
        "DISTINCT_CASE_NAMES_PROVE_SENSITIVITY"
    );
}
#[cfg(unix)]
#[test]
fn filename_case_special_file_answers_as_gnu() {
    use std::os::unix::net::UnixListener;
    let fixture = Fixture::new();
    let _listener = UnixListener::bind(fixture.0.join("socket")).unwrap();
    let mut vm = Vm::new();
    assert_eq!(
        fixture.call(&mut vm, "socket"),
        "nil"
    );
}
/// No directory is listed, so a directory beyond any listing bound answers
/// as an ordinary one does; on a filesystem that does not fold case the
/// measurement is GNU's nil.
#[test]
fn filename_case_matches_gnu_beside_a_huge_directory() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("a"), b"unchanged").unwrap();
    let huge = fixture.0.join("huge");
    std::fs::create_dir(&huge).unwrap();
    for index in 0..4100 {
        std::fs::write(huge.join(format!("entry-{index}")), b"").unwrap();
    }
    let names = ["", "a", "missing", "huge", "huge/entry-7", "huge/missing", "huge/missing/nested"];
    let calls: Vec<String> = names.iter().map(|name| format!(
        "(condition-case e (file-name-case-insensitive-p {}) (error e))",
        serde_json::to_string(&fixture.0.join(name)).unwrap())).collect();
    let source = format!("(list {})", calls.join(" "));
    let output = std::process::Command::new(pinned_gnu())
        .args(["--batch", "-Q", "--eval", &format!("(prin1 {source})")])
        .output().expect("pinned GNU is required");
    assert!(output.status.success(), "GNU oracle failed: {output:?}");
    let expected = String::from_utf8(output.stdout).unwrap();
    assert_eq!(expected, "(nil nil nil nil nil nil nil)", "GNU_LINUX_ANSWERS_NIL");
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, &source), expected, "HUGE_DIRECTORY_ANSWERS_AS_GNU");
    // A relative name under the huge directory, as startup asks.
    let relative = format!(
        "(let ((default-directory {})) (file-name-case-insensitive-p \"entry-7\"))",
        serde_json::to_string(&format!("{}/", huge.display())).unwrap());
    assert_eq!(eval(&mut vm, &relative), "nil", "RELATIVE_UNDER_HUGE_DIRECTORY");
}
/// A real casefold directory, made without root: a tmpfs mounted with
/// casefold in a private user and mount namespace (Linux 6.13), where this
/// test runs itself again. A `+F' directory, its files and its missing
/// names fold case; a plain directory beside it and the mount's root do
/// not. Pinned GNU assumes nil for all of them (VM2-SAVE-CASE).
#[test]
fn filename_case_measures_a_real_casefold_directory() {
    const NAME: &str = "filename_case_measures_a_real_casefold_directory";
    let Some(mount) = std::env::var_os("ROMACS_CASEFOLD_MOUNT") else {
        let fixture = Fixture::new();
        let output = std::process::Command::new("unshare")
            .args(["-rm", "sh", "-c", "mount -t tmpfs -o casefold=utf8-12.1.0 none \"$1\" && exec \"$2\" --exact \"$3\" --nocapture", "sh"])
            .arg(&fixture.0).arg(std::env::current_exe().unwrap()).arg(NAME)
            .env("ROMACS_CASEFOLD_MOUNT", &fixture.0)
            .output().expect("unshare is required");
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        assert!(output.status.success() && text(&output.stdout).contains("1 passed"),
                "casefold namespace run failed: {}\n{}", text(&output.stdout), text(&output.stderr));
        return;
    };
    use std::os::fd::AsRawFd;
    let mount = std::path::PathBuf::from(mount);
    let (fold, plain) = (mount.join("fold"), mount.join("plain"));
    std::fs::create_dir(&fold).unwrap();
    std::fs::create_dir(&plain).unwrap();
    let directory = std::fs::File::open(&fold).unwrap();
    let mut flags: libc::c_int = 0;
    assert_eq!(unsafe { libc::ioctl(directory.as_raw_fd(), libc::FS_IOC_GETFLAGS, &mut flags) }, 0);
    flags |= 0x4000_0000;
    assert_eq!(unsafe { libc::ioctl(directory.as_raw_fd(), libc::FS_IOC_SETFLAGS, &flags) }, 0, "chattr +F");
    std::fs::write(fold.join("Abc"), b"").unwrap();
    assert!(fold.join("ABC").exists(), "the directory folds case");
    let names = [fold.clone(), fold.join("Abc"), fold.join("missing/nested"), plain.clone(), plain.join("x"), mount.clone()];
    let calls: Vec<String> = names.iter().map(|name| format!(
        "(condition-case e (file-name-case-insensitive-p {}) (error e))",
        serde_json::to_string(name).unwrap())).collect();
    let source = format!("(list {})", calls.join(" "));
    let output = std::process::Command::new(pinned_gnu())
        .args(["--batch", "-Q", "--eval", &format!("(prin1 {source})")])
        .output().expect("pinned GNU is required");
    assert!(output.status.success(), "GNU oracle failed: {output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "(nil nil nil nil nil nil)", "GNU_ASSUMES_NIL");
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, &source), "(t t t nil nil nil)", "CASEFOLD_MEASURED");
}
#[test]
fn filename_case_handler_gets_expanded_name() {
    let mut vm = Vm::new();
    eval(&mut vm, "(setq default-directory \"/tmp/\")");
    eval(&mut vm, "(defun sj-handler (op name) (list op name))");
    eval(
        &mut vm,
        "(put 'sj-handler 'operations '(file-name-case-insensitive-p))",
    );
    assert_eq!(
        eval(
            &mut vm,
            "(let ((default-directory \"/tmp/\") (file-name-handler-alist (list (cons \"case-probe\" 'sj-handler)))) (file-name-case-insensitive-p \"case-probe\"))"
        ),
        "(file-name-case-insensitive-p \"/tmp/case-probe\")"
    );
}
#[test]
fn file_attributes_contract() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let file = fixture.0.join("attrs");
    std::fs::write(&file, b"abc").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    let stamp = std::time::UNIX_EPOCH + std::time::Duration::new(123, 456);
    std::fs::File::options().write(true).open(&file).unwrap().set_times(std::fs::FileTimes::new().set_accessed(stamp).set_modified(stamp)).unwrap();
    let mut vm = Vm::new();
    eval(&mut vm, "(setq current-time-list t)");
    assert_eq!(eval(&mut vm, &format!("(let ((a (file-attributes {} 'integer))) (list (nth 4 a) (nth 5 a)))", serde_json::to_string(&file).unwrap())), "((0 123 0 456000) (0 123 0 456000))", "ATTRIBUTE_TIME_NANOSECONDS");
    assert_eq!(eval(&mut vm, &format!("(let ((a (file-attributes {} 'integer))) (list (car a) (nth 1 a) (nth 7 a) (equal (nth 8 a) \"-rw-r-----\") (multibyte-string-p (nth 8 a)) (nth 9 a) (length a)))", serde_json::to_string(&file).unwrap())), "(nil 1 3 t nil t 12)", "FILE_ATTRIBUTES_FIELDS");
    assert_eq!(eval(&mut vm, "(file-attributes 2)"), "nil");
    eval(&mut vm, "(defun sj-attrs (&rest args) args)");
    eval(&mut vm, "(put 'sj-attrs 'operations '(file-attributes))");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . sj-attrs)))) (list (file-attributes \"/remote\") (file-attributes \"/remote\" 'integer)))"), "((file-attributes \"/remote\") (file-attributes \"/remote\" integer))");
}
#[test]
fn file_symlink_contract() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("a"), b"original").unwrap();
    std::os::unix::fs::symlink("missing-target", fixture.0.join("link")).unwrap();
    let mut vm = Vm::new();
    for (name, expected) in [("a", "nil"), ("missing", "nil"), ("link", "\"missing-target\"")] {
        assert_eq!(eval(&mut vm, &format!("(file-symlink-p {})", serde_json::to_string(&fixture.0.join(name)).unwrap())), expected, "SYMLINK_TARGET {name}");
    }
    eval(&mut vm, "(setq default-directory \"/tmp/\")");
    eval(&mut vm, "(defun sj-link (op name) (list op name))");
    eval(&mut vm, "(put 'sj-link 'operations '(file-symlink-p))");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . sj-link)))) (funcall 'file-symlink-p \"remote\"))"), "(file-symlink-p \"/tmp/remote\")");
}
#[test]
fn file_name_nondirectory_contract() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (file-name-nondirectory \"\") (file-name-nondirectory \"/\") (file-name-nondirectory \"//\") (file-name-nondirectory \"a\") (file-name-nondirectory \"a/\") (file-name-nondirectory \"a//b\"))"), "(\"\" \"\" \"\" \"a\" \"\" \"b\")", "NONDIRECTORY_COMPONENT");
    eval(&mut vm, "(defun sj-directory (op name) name)");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . sj-directory)))) (funcall 'file-name-nondirectory \"remote/x\"))"), "\"remote/x\"");
    eval(&mut vm, "(defun sj-directory (op name) 2)");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . sj-directory)))) (condition-case e (file-name-nondirectory \"remote/x\") (error e)))"), "(error \"Invalid handler in ‘file-name-handler-alist’\")");
}
#[test]
fn file_name_as_directory_contract() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (file-name-as-directory \"\") (file-name-as-directory \"/\") (file-name-as-directory \"//\") (file-name-as-directory \"a\") (file-name-as-directory \"a/\") (file-name-as-directory \"a//\"))"), "(\"./\" \"/\" \"//\" \"a/\" \"a/\" \"a//\")", "APPEND_DIRECTORY_SEPARATOR");
    eval(&mut vm, "(defun sj-directory (op name) name)");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . sj-directory)))) (funcall 'file-name-as-directory \"remote\"))"), "\"remote\"");
    eval(&mut vm, "(defun sj-directory (op name) 2)");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . sj-directory)))) (condition-case e (file-name-as-directory \"remote\") (error e)))"), "(error \"Invalid handler in ‘file-name-handler-alist’\")");
}
#[test]
fn directory_file_name_contract() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (directory-file-name \"\") (directory-file-name \"/\") (directory-file-name \"//\") (directory-file-name \"///\") (directory-file-name \"a///\") (directory-file-name \"/a\"))"), "(\"\" \"/\" \"//\" \"/\" \"a\" \"/a\")", "TRAILING_SLASHES");
    eval(&mut vm, "(defun sj-directory (op name) name)");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . sj-directory)))) (funcall 'directory-file-name \"remote/\"))"), "\"remote/\"");
    eval(&mut vm, "(defun sj-directory (op name) 2)");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . sj-directory)))) (condition-case e (directory-file-name \"remote/\") (error e)))"), "(error \"Invalid handler in ‘file-name-handler-alist’\")");
}
#[test]
fn directory_component_and_handler_contract() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (file-name-directory \"\") (file-name-directory \"abc\") (file-name-directory \"a/b\") (file-name-directory \"/\") (file-name-directory \"a//b\") (file-name-directory \"a/\"))"), "(nil nil \"a/\" \"/\" \"a//\" \"a/\")", "DIRECTORY_COMPONENT");
    assert_eq!(eval(&mut vm, "(condition-case e (file-name-directory 2) (error e))"), "(wrong-type-argument stringp 2)");
    eval(&mut vm, "(defun sj-directory (op name) (list op name))");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . sj-directory)))) (file-name-directory \"remote/x\"))"), "nil", "NONSTRING_HANDLER_RESULT");
    eval(&mut vm, "(defun sj-directory (op name) name)");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote\" . sj-directory)))) (funcall 'file-name-directory \"remote/x\"))"), "\"remote/x\"", "RAW_HANDLER_ARGUMENT");
    assert_eq!(eval(&mut vm, "(let* ((s (propertize (unibyte-string 255 47 65) 'face 'bold)) (out (file-name-directory s))) (list (list (aref out 0) (aref out 1)) (multibyte-string-p out) (get-text-property 0 'face out)))"), "((255 47) nil nil)", "DIRECTORY_BYTES_NO_PROPERTIES");
}
#[test]
fn key_description_native_sequence() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (fboundp 'key-description) (equal (funcall 'key-description [8]) \"C-h\"))"), "(t t)");
    assert_eq!(eval(&mut vm, "(list (key-description [8]) (key-description [24 12] [27 120]) (key-description [27 9]) (key-description [27 27]) (key-description [C-M-left f1]) (key-description nil) (condition-case e (key-description 2) (error e)))"), "(\"C-h\" \"M-x C-x C-l\" \"C-M-i\" \"ESC ESC\" \"C-M-<left> <f1>\" \"\" (wrong-type-argument sequencep 2))", "KEY_DESCRIPTION_GNU");
}
#[test]
fn truename_intrinsic_local_lifecycle() {
    let mut vm = Vm::new();
    eval(&mut vm, "(progn (setq sj-b (get-buffer-create \" *sj-local*\")) nil)");
    assert_eq!(eval(&mut vm, "(with-current-buffer sj-b (list buffer-file-truename (local-variable-p 'buffer-file-truename) (setq buffer-file-truename \"/proof\") (progn (kill-all-local-variables) buffer-file-truename) (condition-case e (setq buffer-file-truename 7) (error e))))"), "(nil t \"/proof\" \"/proof\" (wrong-type-argument stringp 7))");
    assert_eq!(eval(&mut vm, "(progn (kill-buffer sj-b) (buffer-local-value 'buffer-file-truename sj-b))"), "\"/proof\"");
    assert_eq!(eval(&mut vm, "(eq command-error-function 'command-error-default-function)"), "t");
}
#[test]
fn truename_buffer_native_lookup() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(get-truename-buffer 2)"), "nil", "NO_STRING_FIELD_NO_ARGUMENT_CHECK");
    eval(&mut vm, "(progn (setq sj-b (get-buffer-create \" *sj-truename*\")) nil)");
    eval(&mut vm, "(with-current-buffer sj-b (make-local-variable 'buffer-file-truename) (setq buffer-file-truename \"/sj-proof\"))");
    assert_eq!(eval(&mut vm, "(list (eq (get-truename-buffer \"/sj-proof\") sj-b) (eq (funcall 'get-truename-buffer (intern \"/sj-proof\")) sj-b) (get-truename-buffer \"/missing\") (condition-case e (get-truename-buffer 2) (error e)))"), "(t t nil (wrong-type-argument stringp 2))", "TRUENAME_NATIVE_LOOKUP");
    eval(&mut vm, "(progn (setq sj-old-list (symbol-function 'buffer-list)) nil)");
    eval(&mut vm, "(progn (fset 'buffer-list (lambda () (error \"mutable Lisp must not own native traversal\"))) nil)");
    assert_eq!(eval(&mut vm, "(eq (get-truename-buffer \"/sj-proof\") sj-b)"), "t");
    eval(&mut vm, "(progn (fset 'buffer-list sj-old-list) nil)");
    assert_eq!(eval(&mut vm, "(kill-buffer sj-b)"), "t");
    assert_eq!(eval(&mut vm, "(list (buffer-live-p sj-b) (buffer-local-value 'buffer-file-truename (current-buffer)) (eq (get-truename-buffer \"/sj-proof\") sj-b))"), "(nil nil nil)");
    assert_eq!(eval(&mut vm, "(get-truename-buffer \"/sj-proof\")"), "nil");
}
#[test]
fn decode_nil_coding_preserves_bytes_properties_and_copy_contract() {
    let mut vm = Vm::new();
    assert_eq!(
        eval(
            &mut vm,
            "(let* ((s (propertize (unibyte-string 65 255 13) 'face 'bold)) (out (decode-coding-string s nil))) (list (equal s out) (eq s out) (multibyte-string-p out) (get-text-property 0 'face out) last-coding-system-used (eq s (decode-coding-string s nil t))))"
        ),
        "(t nil nil bold no-conversion t)",
        "DECODE_NIL_IDENTITY_COPY"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(condition-case e (decode-coding-string 2 nil) (error e))"
        ),
        "(wrong-type-argument stringp 2)"
    );
}
