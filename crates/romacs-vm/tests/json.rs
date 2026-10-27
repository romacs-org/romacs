//! json.c's native JSON functions against pinned GNU: serialization,
//! parse results, every error with its position, argument errors and
//! json-parse-buffer's point.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(json-serialize '((a . 1) (b . "x") (c . [1 2.5 t :null :false]) (d . nil)))
(json-serialize '(:a 1 :b (:c "d") : 3 e 4))
(json-serialize '((a . 1) (a . 2) (b . 3)))
(json-serialize "é\n\t\"\\\x01\x7f/ü😀")
(multibyte-string-p (json-serialize "é"))
(append (json-serialize "é😀") nil)
(json-serialize (list 1.0 -0.0 1e100 0.1 123456789012345678901234567890 -5))
(json-serialize [1.0 -0.0 1e100 0.1 123456789012345678901234567890 -5 1e-7])
(json-serialize 1.0e+INF)
(json-serialize 0.0e+NaN)
(json-serialize 'foo)
(json-serialize '((1 . 2)))
(json-serialize '(a 1 b))
(json-serialize '((a . 1) . 3))
(json-serialize '((a . 1) b))
(json-serialize (string-to-unibyte "\200"))
(json-serialize (string 4194303))
(let ((h (make-hash-table :test 'equal))) (puthash "k" 1 h) (puthash "j" [] h) (json-serialize h))
(let ((h (make-hash-table))) (puthash 'k 1 h) (json-serialize h))
(json-serialize nil)
(json-serialize :null)
(json-serialize [nil t :false] :false-object nil :null-object t)
(json-serialize [nil :false] :null-object nil :null-object 7)
(json-serialize 1 :object-type 'alist)
(json-serialize 1 :null-object)
(let ((v 1) (i 0)) (while (< i 50) (setq v (vector v) i (1+ i))) (json-serialize v))
(let ((v 1) (i 0)) (while (< i 51) (setq v (vector v) i (1+ i))) (json-serialize v))
(let ((l (list (cons 'a 1) (cons 'b 2)))) (setcdr (cdr l) l) (condition-case e (json-serialize l) (error (list (car e) (eq (car (cdr e)) l) (eq (car (cdr e)) (cdr l))))))
(let ((l (list 'a 1 'b 2))) (setcdr (cdr (cdr (cdr l))) l) (condition-case e (json-serialize l) (error (list (car e) (eq (car (cdr e)) l) (eq (car (cdr e)) (cdr (cdr l)))))))
(json-serialize (make-bool-vector 2 t))
(json-serialize '((t . 1) (nil . 2)))
(json-serialize '(:é 1 ::x 2))
(with-temp-buffer (insert "ab") (goto-char 2) (list (json-insert [1 "é"]) (buffer-string) (point)))
(with-temp-buffer (set-buffer-multibyte nil) (json-insert "é") (list (buffer-string) (multibyte-string-p (buffer-string)) (point)))
(with-temp-buffer (insert "ab") (setq buffer-read-only t) (json-insert 1))
(json-parse-string "{\"a\":1,\"b\":[1,2.5,true,false,null],\"c\":{}}" :object-type 'alist)
(json-parse-string "{\"a\":1,\"b\":[1,2.5,true,false,null],\"c\":{}}" :object-type 'plist :array-type 'list)
(json-parse-string "{\"a\":1,\"a\":2,\"b\":3}")
(json-parse-string "{\"a\":1,\"a\":2}" :object-type 'alist)
(json-parse-string "{}")
(json-parse-string "[]")
(json-parse-string "[]" :array-type 'list)
(json-parse-string "{}" :object-type 'plist)
(let ((s (json-parse-string "\"a\""))) (list s (multibyte-string-p s)))
(let ((s (json-parse-string "\"\""))) (list s (multibyte-string-p s)))
(json-parse-string "\"\\u00e9\\ud83d\\ude00\\n\\/\\b\\f\\r\\t\\\"\\\\\"")
(json-parse-string "  -0 ")
(json-parse-string "-0.0")
(json-parse-string "[1e5,1E+5,1.5e-3,-12,123456789012345678901234567890,-9223372036854775808,18446744073709551615,18446744073709551616,1e400,1e-400]")
(json-parse-string "1e400")
(json-parse-string "01")
(json-parse-string "[01]")
(json-parse-string "1.")
(json-parse-string "1.e5")
(json-parse-string "1e")
(json-parse-string "-")
(json-parse-string "-a")
(json-parse-string "tru")
(json-parse-string "truex")
(json-parse-string "true ")
(json-parse-string "true x")
(json-parse-string "nul")
(json-parse-string "fals")
(json-parse-string "")
(json-parse-string "   \n  ")
(json-parse-string "[1,\n2,\n]")
(json-parse-string "{\"a\" 1}")
(json-parse-string "{a:1}")
(json-parse-string "{\"a\":1,}")
(json-parse-string "[1 2]")
(json-parse-string "\"abc")
(json-parse-string "\"a\nb\"")
(json-parse-string "\"\\x\"")
(json-parse-string "\"\\u12G4\"")
(json-parse-string "\"\\ud800\"")
(json-parse-string "\"\\ud800\\u0041\"")
(json-parse-string "\"\\udc00\"")
(json-parse-string "\"é\" é")
(json-parse-string (string-to-unibyte "\"\303\251\""))
(json-parse-string (string-to-unibyte "\"\303\""))
(json-parse-string (string-to-unibyte "\"\355\240\200\""))
(json-parse-string (string-to-unibyte "\"\300\200\""))
(json-parse-string (string-to-unibyte "\"\370\200\200\200\200\""))
(json-parse-string (concat "\"" (string 4194303) "\""))
(json-parse-string "[1]" :array-type 'vector)
(json-parse-string "[1]" :object-type 'alist :object-type 'plist)
(json-parse-string "[1]" :bogus 1)
(json-parse-string "[1]" :null-object)
(json-parse-string 'a)
(json-parse-string "[null,false]" :null-object 'N :false-object 'F)
(let ((v (json-parse-string (concat (make-string 10000 ?\[) (make-string 10000 ?\]))))) (let ((n 0)) (while (and (vectorp v) (> (length v) 0)) (setq v (aref v 0) n (1+ n))) (list n v)))
(let ((v (json-parse-string (concat (make-string 10001 ?\[) (make-string 10001 ?\]))))) (let ((n 0)) (while (and (vectorp v) (> (length v) 0)) (setq v (aref v 0) n (1+ n))) (list n v)))
(json-parse-string (concat (make-string 10002 ?\[) (make-string 10002 ?\])) :array-type 'list)
(json-parse-string (concat (make-string 10002 ?\[) (make-string 10002 ?\])))
(json-parse-string "{\"é\":{\"k\":[]}}" :object-type 'plist)
(json-parse-string "{\"\":1}" :object-type 'alist)
(with-temp-buffer (insert "  [1, \"é\"]  {\"a\":2} x") (goto-char 1) (list (json-parse-buffer) (point) (json-parse-buffer :object-type 'alist) (point) (condition-case e (json-parse-buffer) (error e)) (point)))
(with-temp-buffer (insert "é\n[1,\n x]") (goto-char 2) (list (condition-case e (json-parse-buffer) (error e)) (point)))
(with-temp-buffer (list (condition-case e (json-parse-buffer) (error e)) (point)))
(with-temp-buffer (insert "1 2") (goto-char 1) (list (json-parse-buffer) (point)))
(with-temp-buffer (insert "true") (goto-char 1) (list (json-parse-buffer) (point)))
(with-temp-buffer (insert "[1] [2]") (narrow-to-region 1 4) (goto-char 1) (list (json-parse-buffer) (point) (condition-case e (json-parse-buffer) (error e))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "\"\303\251\" 1") (goto-char 1) (list (json-parse-buffer) (point)))
(json-serialize (list (cons 'role (symbol-name 'pair))))
(json-serialize (vector (base64-encode-string "hi") (symbol-name 'a)))
(let ((h (make-hash-table :test 'equal))) (puthash (symbol-name 'k) (symbol-name 'v) h) (json-serialize h))
(json-parse-string (symbol-name 'true))
"###;

#[test]
fn json_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        // As character codes: serialized text is unibyte UTF-8.
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
