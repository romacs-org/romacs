//! `romacs--scanner-run` (prims/scanner.rs): a table tabulated from a Lisp
//! step function, run over buffer text. The Lisp side and its tests are
//! lisp/romacs/romacs-scanner.el and test/lisp/romacs-scanner-tests.el.
use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(romacs_vm::Local::display).unwrap_or_else(|e| panic!("{source}: {e}"))
}

/// Two states, 129 entries each: in state 0 a semicolon is marked and a
/// double quote goes to state 1; in state 1 a double quote goes back.
const TABLE: &str = "(let ((table (make-vector 258 0)) (i 129)) \
    (while (< i 258) (aset table i 1) (setq i (1+ i))) \
    (aset table ?\\; 128) (aset table ?\\\" 1) (aset table (+ 129 ?\\\") 0) table)";

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    eval(&mut vm, &format!("(defvar scanner-table {TABLE})"));
    vm
}

#[test]
fn a_scan_reports_the_marked_characters_and_the_last_state() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(with-temp-buffer (insert \"a;\\\"b;\\\";c;\\\"d\") \
        (list (romacs--scanner-run scanner-table 0 1 (point-max) nil) (romacs--scanner-run scanner-table 0 1 (point-max) t) \
              (romacs--scanner-run scanner-table 1 4 8 nil) (romacs--scanner-run scanner-table 0 5 5 nil)))"),
        "((1 2 7 9) (1 . 3) (0 7) (0))");
    // A region that does not hold point: an empty one at the end, and a
    // suffix, with point at the start; then the whole text with point in
    // the middle of a large buffer.
    assert_eq!(eval(&mut vm, "(with-temp-buffer (insert \"a;\\\"b;\\\";c;\\\"d\") (goto-char 1) \
        (list (romacs--scanner-run scanner-table 0 (point-max) (point-max) nil) (romacs--scanner-run scanner-table 0 7 (point-max) nil)))"),
        "((0) (1 7 9))");
    assert_eq!(eval(&mut vm, "(with-temp-buffer (dotimes (_ 4000) (insert \"ab;cd\\n\")) (goto-char 12000) (insert \"x\") (goto-char 3) \
        (list (cdr (romacs--scanner-run scanner-table 0 1 (point-max) t)) (cdr (romacs--scanner-run scanner-table 0 20000 (point-max) t))))"),
        "(4000 667)");
}

#[test]
fn positions_count_characters_in_multibyte_text_and_bytes_in_unibyte() {
    let mut vm = vm();
    // Every character past ASCII takes the last entry of its state's row,
    // once, however many bytes it is.
    assert_eq!(eval(&mut vm, "(with-temp-buffer (insert \"é;\\\"日;\\\";λ;\") (romacs--scanner-run scanner-table 0 1 (point-max) nil))"),
        "(0 2 7 9)");
    assert_eq!(eval(&mut vm, "(with-temp-buffer (set-buffer-multibyte nil) (insert \"\\351;\\\"\\200;\\\";\\277;\") \
        (romacs--scanner-run scanner-table 0 1 (point-max) nil))"),
        "(0 2 7 9)");
}

#[test]
fn a_scan_stays_within_the_accessible_text_and_a_sound_table() {
    let mut vm = vm();
    let error = |vm: &mut Vm, form: &str| eval(vm, &format!("(with-temp-buffer (insert \"abc;def\") (narrow-to-region 2 6) (condition-case e {form} (error (car e))))"));
    assert_eq!(error(&mut vm, "(romacs--scanner-run scanner-table 0 2 6 nil)"), "(0 4)");
    assert_eq!(error(&mut vm, "(romacs--scanner-run scanner-table 0 1 6 nil)"), "args-out-of-range");
    assert_eq!(error(&mut vm, "(romacs--scanner-run scanner-table 0 2 7 nil)"), "args-out-of-range");
    assert_eq!(error(&mut vm, "(romacs--scanner-run scanner-table 0 5 3 nil)"), "args-out-of-range");
    assert_eq!(error(&mut vm, "(romacs--scanner-run scanner-table 2 2 6 nil)"), "wrong-type-argument");
    assert_eq!(error(&mut vm, "(romacs--scanner-run (make-vector 130 0) 0 2 6 nil)"), "wrong-type-argument");
    // An entry naming a state the table does not have.
    assert_eq!(error(&mut vm, "(romacs--scanner-run (make-vector 129 5) 0 2 6 nil)"), "wrong-type-argument");
    assert_eq!(error(&mut vm, "(romacs--scanner-run scanner-table 0 2.0 6 nil)"), "wrong-type-argument");
}

/// A scan costs a table lookup a byte. Pinned at a tenth of what GNU's
/// native compiler spends on the same machine written as an inlined loop
/// (147 instructions a character, measured on pinned GNU at speed 2): the
/// reason to write a scanner as a step function.
#[test]
fn a_scan_costs_a_tenth_of_the_loop_in_gnus_native_code() {
    let mut vm = vm();
    let mut cost = |lines: usize, count: &str| -> f64 {
        let form = format!("(with-temp-buffer (dotimes (_ {lines}) (insert \"int x = f(a, \\\"b;\\\"); /* c; */ y = 2;\\n\")) \
            (romacs--scanner-run scanner-table 0 1 2 {count}) \
            (let ((a (romacs-instructions))) (romacs--scanner-run scanner-table 0 1 (point-max) {count}) (and a (- (romacs-instructions) a))))");
        let text = eval(&mut vm, &form);
        text.parse().unwrap_or_else(|_| panic!("no instruction counter (romacs-instructions gave {text})"))
    };
    // 36 characters a line, three of them marked.
    let counting = (cost(10_000, "t") - cost(2_500, "t")) / (7_500.0 * 36.0);
    assert!(counting < 14.7, "a scan costs {counting:.1} instructions a character");
    // Reporting a position adds its cons and the count of characters since
    // the last one.
    let reporting = (cost(10_000, "nil") - cost(2_500, "nil")) / (7_500.0 * 36.0);
    let a_position = (reporting - counting) * 12.0;
    // 10.2 a character and 186 a position when written.
    assert!(a_position < 260.0, "a reported position costs {a_position:.0} instructions");
}
