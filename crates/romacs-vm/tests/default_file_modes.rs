//! `default-file-modes` and `set-default-file-modes` (fileio.c): the process
//! umask, which is process-wide, so this target runs alone in its process.
use romacs_vm::Vm;
use std::os::unix::fs::PermissionsExt;

fn eval(vm: &mut Vm, text: &str) -> String {
    vm.eval_source(text)
        .map(|v| v.display().unwrap())
        .unwrap_or_else(|e| format!("ERROR {e}"))
}

/// Pinned GNU: the modes round-trip through the mask, a directory made
/// meanwhile gets them (files.el `with-file-modes', as startup's
/// `locate-user-emacs-file' creates the user directory), and a mode must be
/// a fixnum.
#[test]
fn default_file_modes_set_the_mask_new_files_get() {
    let dir = std::env::temp_dir().join(format!("romacs-default-file-modes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let made = serde_json::to_string(dir.join("made").to_str().unwrap()).unwrap();
    let mut vm = Vm::new();
    let original = eval(&mut vm, "(default-file-modes)");
    let set = eval(&mut vm, &format!(
        "(let ((old (default-file-modes)))
           (unwind-protect
               (list (set-default-file-modes #o700) (default-file-modes)
                     (progn (make-directory-internal {made}) (file-modes {made})))
             (set-default-file-modes old)))"));
    assert_eq!(set, "(nil 448 448)");
    assert_eq!(eval(&mut vm, "(default-file-modes)"), original, "restored");
    assert_eq!(std::fs::metadata(dir.join("made")).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(eval(&mut vm, "(condition-case err (set-default-file-modes \"700\") (error err))"),
               r#"(wrong-type-argument fixnump "700")"#);
    std::fs::remove_dir_all(&dir).unwrap();
}
