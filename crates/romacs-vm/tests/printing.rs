use romacs_vm::Vm;
#[test]
fn quote_nesting_is_bounded_in_child_process() {
 for shape in ["cycle", "deep"] {
  let status=std::process::Command::new(std::env::current_exe().unwrap())
   .args(["--exact", "quote_nesting_child", "--nocapture"])
   .env("ROMACS_PRINT_DEPTH_CHILD",shape).status().unwrap();
  assert!(status.success(), "quote nesting must signal instead of aborting: {shape}: {status}");
 }
}
#[test]
fn quote_nesting_child() {
 let Ok(shape)=std::env::var("ROMACS_PRINT_DEPTH_CHILD") else { return; };
 let mut vm=Vm::new();
 let source=if shape=="cycle" { "(let ((x (list 'quote nil))) (setcar (cdr x) x) (prin1-to-string x))" }
 else { "(let ((x 'a) (i 0)) (while (< i 5000) (setq x (list 'quote x) i (1+ i))) (prin1-to-string x))" };
 assert!(vm.eval_source(source).is_err(), "unsupported quote depth must refuse");
}
#[test]
fn printing_inventory() {
 let mut vm=Vm::new();
 for name in ["prin1", "princ", "prin1-to-string", "terpri", "print", "write-char", "flush-standard-output", "print--preprocess"] {
  let source=format!("(list (fboundp '{name}) (functionp '{name}))");
  println!("{name}: {}",vm.eval_source(&source).unwrap().display().unwrap());
 }
}
fn check(source: &str, expected: &str) {
 let mut vm=Vm::new();
 assert_eq!(vm.eval_source(source).map(|v|v.display().unwrap()),Ok(expected.into()),"{source}");
}
#[test]
fn princ_unquoted_character_stream() {
 check("(let ((out nil)) (list (princ \"λ\\n\" (lambda (c) (setq out (cons c out)))) (nreverse out)))", "(\"λ\\n\" (955 10))");
}
#[test]
fn terpri_newline_return_and_ensure_error() {
 check("(let ((out nil)) (list (terpri (lambda (c) (setq out (cons c out)))) out))", "(t (10))");
 check("(condition-case e (terpri (lambda (c) nil) t) (error (car e)))", "error");
}
#[test]
fn print_delimits_quoted_object() {
 check("(let ((out nil)) (list (print \"x\" (lambda (c) (setq out (cons c out)))) (nreverse out)))", "(\"x\" (10 34 120 34 10))");
}
#[test]
fn write_char_emits_one_character_and_returns_it() {
 check("(let ((out nil)) (list (write-char 955 (lambda (c) (setq out (cons c out)))) out))", "(955 (955))");
 check("(condition-case e (write-char 'bad (lambda (c) nil)) (error e))", "(wrong-type-argument fixnump bad)");
}
#[test]
fn flush_standard_output_returns_nil() { check("(flush-standard-output)", "nil"); }
#[test]
fn printer_controls() {
 check("(let ((print-length 1)) (prin1-to-string '(1 2 3)))", "\"(1 ...)\"");
 check("(let ((print-level 1)) (prin1-to-string '(1 (2 (3)))))", "\"(1 ...)\"");
 check("(let ((print-quoted t)) (prin1-to-string '(quote x)))", "\"'x\"");
 check("(let ((print-escape-nonascii t)) (prin1-to-string (unibyte-string 255)))", "\"\\\"\\\\377\\\"\"");
}
#[test]
fn printer_circle_shared_and_self() {
 check("(let ((print-circle t) (x (list 'a))) (prin1-to-string (list x x)))", "\"(#1=(a) #1#)\"");
 check("(let ((print-circle t) (x (list 'a))) (setcdr x x) (prin1-to-string x))", "\"#1=(a . #1#)\"");
}
#[test]
fn output_functions_are_indirectly_callable() {
 check("(let* ((out nil) (sink (lambda (c) (setq out (cons c out))))) (list (funcall #'prin1 7 sink) (funcall #'princ 8 sink) (funcall #'print 9 sink) (funcall #'terpri sink) (funcall #'write-char 955 sink) (nreverse out)))", "(7 8 9 t 955 (55 56 10 57 10 10 955))");
}
#[test]
fn prin1_quotes_and_returns_input() {
 let mut vm=Vm::new();
 let got=vm.eval_source("(let ((out nil)) (let ((x (list \"λ\\n\"))) (list (eq x (prin1 x (lambda (c) (setq out (cons c out))))) (nreverse out))))").map(|v|v.display().unwrap());
 assert_eq!(got,Ok("(t (40 34 955 10 34 41))".into()),"prin1 quotes strings, emits characters, and returns original object");
}
