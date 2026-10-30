//! sqlite.c's primitives against pinned GNU: databases, binding, row
//! conversion, result sets, transactions and every signal path gnosis and
//! multisession can reach. Object printing is left out: it shows addresses.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(let ((db (sqlite-open))) (list (sqlitep db) (type-of db) (sqlitep 1) (sqlite-available-p) (stringp (sqlite-version)) (sqlite-execute db "create table t (a, b)") (sqlite-execute db "insert into t values (?, ?)" '(1 "é")) (sqlite-execute db "insert into t values (?, ?)" [2.5 nil]) (sqlite-select db "select * from t") (sqlite-select db "select * from t" nil 'full) (sqlite-select db "select a from t where a > ?" '(1)) (sqlite-execute db "select a from t") (sqlite-execute db "update t set b = 'x'") (sqlite-close db) (sqlite-close db)))
(let ((db (sqlite-open))) (sqlite-execute db "create table t (a)") (sqlite-execute db "insert into t values (1), (2)") (let ((set (sqlite-select db "select a from t" nil 'set))) (list (sqlite-columns set) (sqlite-more-p set) (sqlite-next set) (sqlite-next set) (sqlite-more-p set) (sqlite-next set) (sqlite-more-p set) (sqlite-next set) (sqlite-finalize set) (condition-case e (sqlite-next set) (error e)) (condition-case e (sqlite-more-p set) (error e)) (condition-case e (sqlite-columns set) (error e)) (sqlite-close set) (condition-case e (sqlite-finalize set) (error e)))))
(let* ((db (sqlite-open)) (set (sqlite-select db "select 1" nil 'set))) (list (condition-case e (sqlite-next db) (error e)) (condition-case e (sqlite-execute set "select 1") (error e)) (condition-case e (sqlite-close set) (error e)) (sqlite-close db) (sqlite-next set) (sqlite-next set) (condition-case e (sqlite-execute db "select 1") (error e)) (condition-case e (sqlite-select db "select 1") (error e)) (condition-case e (sqlite-transaction db) (error e)) (condition-case e (sqlite-close 1) (error e)) (condition-case e (sqlite-next nil) (error e))))
(let ((db (sqlite-open))) (list (condition-case e (sqlite-execute db "bogus") (error e)) (condition-case e (sqlite-select db "select * from missing") (error e)) (condition-case e (sqlite-execute db "select 1" "x") (error e)) (condition-case e (sqlite-select db "select 1" 1) (error e)) (condition-case e (sqlite-execute db 'x) (error e)) (condition-case e (sqlite-execute db "select ?" '(foo)) (error e)) (condition-case e (sqlite-execute db "select ?" '(1 2)) (error e)) (condition-case e (sqlite-execute db "select ?" '(1 . 2)) (error e)) (condition-case e (sqlite-execute db "select ?" '(1 2 foo)) (error e)) (condition-case e (let ((l (list 1 2))) (setcdr (cdr l) l) (sqlite-execute db "select ?" l)) (error (car e))) (condition-case e (sqlite-execute db "create table u (a primary key)") (error e)) (condition-case e (sqlite-execute db "insert into u values (1), (1)") (error e)) (sqlite-select db "select count(*) from u")))
(let ((db (sqlite-open))) (sqlite-select db "select ?, ?, ?, ?, ?, ?, ?, ?, typeof(?), typeof(?)" (list t 'false nil 1.5 (expt 2 62) (expt 2 70) (- (expt 2 63)) "" "" (propertize "ab" 'coding-system 'binary))))
(let ((db (sqlite-open)) (last-coding-system-used 'keep)) (list (sqlite-select db "select ?, length(?), hex(?), typeof(?)" (list "λ" "λ" "λ" "λ")) (sqlite-select db "select hex(?), typeof(?)" (list (string-to-unibyte "\377a") (propertize (string-to-unibyte "\377a") 'coding-system 'binary))) (sqlite-select db "select hex(?)" (list (string 4194303 ?a))) (condition-case e (sqlite-select db "select ?" (list (propertize "λ" 'coding-system 'binary))) (error e)) last-coding-system-used))
(let ((db (sqlite-open)) (last-coding-system-used 'keep)) (list (sqlite-select db "select cast(x'ff41' as text), 'a' || char(13) || char(10) || 'b', x'00ff', 'é', 9223372036854775807, -9223372036854775808, 1e300, null") (multibyte-string-p (car (car (sqlite-select db "select 'abc'")))) (multibyte-string-p (car (car (sqlite-select db "select x'41'")))) last-coding-system-used))
(let ((db (sqlite-open))) (sqlite-execute db "create table t (a)") (list (sqlite-transaction db) (sqlite-execute db "insert into t values (1)") (sqlite-rollback db) (sqlite-select db "select * from t") (sqlite-transaction db) (sqlite-execute db "insert into t values (2)") (sqlite-commit db) (sqlite-select db "select * from t") (sqlite-commit db) (sqlite-rollback db) (sqlite-execute-batch db "insert into t values (3); insert into t values (4);") (sqlite-execute-batch db "insert into t values (5); bogus;") (sqlite-select db "select * from t") (sqlite-execute db "insert into t values (6); insert into t values (7)") (sqlite-select db "select count(*) from t") (sqlite-pragma db "user_version = 7") (sqlite-select db "pragma user_version") (sqlite-pragma db "bogus syntax (") (condition-case e (sqlite-pragma db 1) (error e)) (condition-case e (sqlite-execute-batch db 1) (error e))))
(let ((db (sqlite-open))) (list (condition-case e (sqlite-load-extension db "foo.so") (error e)) (condition-case e (sqlite-load-extension db "/nonexistent/libsqlite3_mod_csv.so") (error e)) (condition-case e (sqlite-load-extension db "/nonexistent/rtree.dll2") (error e)) (condition-case e (sqlite-load-extension db 1) (error e)) (condition-case e (sqlite-load-extension 1 "csv.so") (error e))))
(let* ((file "TMPDIR/romacs-sqlite-oracle-test.db") (_ (when (file-exists-p file) (delete-file file))) (db (sqlite-open file))) (list (sqlite-execute db "create table t (a)") (sqlite-execute db "insert into t values ('kept')") (sqlite-close db) (let ((ro (sqlite-open file t))) (prog1 (list (sqlite-select ro "select * from t") (condition-case e (sqlite-execute ro "insert into t values (1)") (error e))) (sqlite-close ro))) (sqlite-open "/nonexistent-romacs-dir/x.db") (progn (delete-file file) (file-exists-p file))))
(let ((db (sqlite-open))) (sqlite-select db "select compile_options from pragma_compile_options where compile_options like 'MAX_VARIABLE_NUMBER=%'"))
"###;

#[test]
fn sqlite_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    let tmpdir = std::env::var("TMPDIR").expect("TMPDIR for the file database");
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        let expression = expression.replace("TMPDIR/", &format!("{tmpdir}/"));
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = Vm::new().eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
