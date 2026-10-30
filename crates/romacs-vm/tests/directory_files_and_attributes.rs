// em-ls.el's `eshell-ls--insert-directory' lists a directory with
// `directory-files-and-attributes' (GNU dired.c:420).
use romacs_vm::Vm;
use std::process::Command;

#[test]
fn directory_files_and_attributes_matches_gnu() {
    let root = std::env::temp_dir().join(format!("romacs-dfa-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), "yy").unwrap();
    std::fs::write(root.join("b.txt"), "x").unwrap();
    std::os::unix::fs::symlink("missing", root.join("dangling")).unwrap();
    let directory = serde_json::to_string(root.to_str().unwrap()).unwrap();
    let source = format!(
        "(let ((d {directory})) (list (mapcar (lambda (e) (list (car e) (nth 0 (cdr e)) (nth 7 (cdr e)) (nth 8 (cdr e)))) (directory-files-and-attributes d nil \"^[^.]\")) (mapcar 'car (directory-files-and-attributes (concat d \"/\") t \"a\\\\.txt\")) (equal (cdr (assoc \"a.txt\" (directory-files-and-attributes d))) (file-attributes (expand-file-name \"a.txt\" d))) (equal (cdr (assoc \"sub\" (directory-files-and-attributes d nil nil nil 'string))) (file-attributes (expand-file-name \"sub\" d) 'string)) (condition-case e (directory-files-and-attributes (concat d \"/nope\")) (file-missing (car e))) (sort (mapcar 'car (directory-files-and-attributes d nil nil t)) 'string-lessp) (length (directory-files-and-attributes d nil nil nil nil 2))))"
    );
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let dir = root.to_str().unwrap();
    let expected = format!(
        "(((\"a.txt\" nil 2 \"-rw-r--r--\") (\"b.txt\" nil 1 \"-rw-r--r--\") (\"dangling\" \"missing\" 7 \"lrwxrwxrwx\") (\"sub\" t {} \"drwxr-xr-x\")) (\"{dir}/a.txt\") t t file-missing (\".\" \"..\" \"a.txt\" \"b.txt\" \"dangling\" \"sub\") 2)",
        std::fs::symlink_metadata(root.join("sub")).map(|m| std::os::unix::fs::MetadataExt::size(&m)).unwrap()
    );
    if let Ok(oracle) = std::env::var("ROMACS_GNU_ORACLE") {
        let output = Command::new(oracle).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "GNU failed: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "pinned GNU");
    }
    let same = format!("(equal (prin1-to-string {source}) {expected:?})");
    assert_eq!(vm.eval_source(&same).unwrap_or_else(|e| panic!("{source}: {e}")).display().unwrap(), "t",
               "candidate: {}", vm.eval_source(&source).unwrap().display().unwrap());
    std::fs::remove_dir_all(root).unwrap();
}
