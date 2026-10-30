//! lread.c `#N=' and `#N#': a label stands for its object while that object
//! is read, so the object may contain itself; and `quote' returns such an
//! object as it was read.
use romacs_vm::Vm;

#[test]
fn self_referential_labels_match_gnu() {
    let cases = [
        r##"(let ((l (car (read-from-string "#1=(1 . #1#)")))) (list (car l) (eq (cdr l) l)))"##,
        r##"(let ((v (car (read-from-string "#1=[a #1#]")))) (list (aref v 0) (eq (aref v 1) v)))"##,
        r##"(let ((x (car (read-from-string "(#1=(a) #1#)")))) (eq (car x) (car (cdr x))))"##,
        r##"(condition-case e (read-from-string "#1=#1#") (error e))"##,
        r##"(let ((l '#1=(1 . #1#))) (list (car l) (eq (cdr l) l)))"##,
        r##"(let ((x '#2=([y] ((y #2#))))) (eq (car (cdr (car (car (cdr x))))) x))"##,
        // hash_table_from_plist's FOR_EACH_TAIL refuses data that reaches itself.
        r##"(condition-case e (progn (read "#s(hash-table data #0=(a b . #0#))") nil) (error (list (car e) (car (car (cdr e))))))"##,
    ];
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    for source in cases {
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        let actual = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(actual.as_deref(), Ok(expected.as_str()), "{source}");
    }
}
