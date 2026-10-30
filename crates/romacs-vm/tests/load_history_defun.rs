//! `defalias' records (defun . SYMBOL) in the loading file's history
//! (data.c defalias, LOADHIST_ATTACH), so `symbol-file' names the file that
//! defined a function, as pinned GNU does.  `autoload' and `defvar' attach
//! nothing yet (DIVERGENCES.md VM2-LOADHIST1).
use romacs_vm::Vm;

#[test]
fn defalias_attaches_defun_entries_like_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("load-history-defun-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lh-defs.el");
    std::fs::write(&file, "(defalias 'lh-one (lambda () 1))\n(defalias 'lh-two 'lh-one)\n").unwrap();
    let path = serde_json::to_string(&file.to_string_lossy()).unwrap();
    let source = format!(
        "(progn (load {path} nil t)
                (let ((entry (cdr (assoc {path} load-history))))
                  (prin1-to-string (list (and entry t)
                                         (and (member '(defun . lh-one) entry) t)
                                         (and (member '(defun . lh-two) entry) t)))))");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    let actual = vm.eval_source(&source).and_then(|v| v.display()).map(|s| serde_json::from_str::<String>(&s).unwrap_or(s));
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(actual.as_deref(), Ok(expected.as_str()));
}
