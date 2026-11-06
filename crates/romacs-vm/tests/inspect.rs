//! `romacs--inspect-frames' (inspect.rs): the call stack as JSON, in bounds
//! whatever the arguments hold, with secrets kept out.
use romacs_vm::Vm;
use serde_json::Value as Json;

fn snapshot(setup: &str, form: &str) -> Json {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(setup).unwrap();
    let text = vm.eval_source(form).unwrap().display().unwrap();
    let json: String = serde_json::from_str(&text).unwrap();
    serde_json::from_str(&json).unwrap_or_else(|error| panic!("{error}: {json}"))
}

const SHOW: &str = "(defun i-show (&rest _) (romacs--inspect-frames))";

/// The frame of the Lisp function that asked: the primitive's own comes first.
fn shown(json: &Json) -> &Json { &json["frames"][1] }
fn args(json: &Json) -> &Json { &shown(json)["args"] }

#[test]
fn frames_name_the_calls_innermost_first() {
    let json = snapshot("(progn (defun i-inner (a b) (list (romacs--inspect-frames))) (defun i-outer (a) (car (i-inner a 2))))", "(i-outer 1)");
    let names: Vec<&str> = json["frames"].as_array().unwrap().iter().filter_map(|f| f["fn"].as_str()).collect();
    let (inner, outer) = (names.iter().position(|n| *n == "i-inner"), names.iter().position(|n| *n == "i-outer"));
    assert!(inner.unwrap() < outer.unwrap(), "{names:?}");
    let frame = &json["frames"].as_array().unwrap()[inner.unwrap()];
    assert_eq!(frame["argc"], 2);
    assert_eq!(frame["args"][1], 2);
    assert!(json["depth"].as_u64().unwrap() >= 2);
}

#[test]
fn values_encode_as_their_kinds() {
    let json = snapshot(SHOW, r#"(i-show nil t 7 1.5 'sym "text" '(1 2 . 3) [a b] (current-buffer))"#);
    let a = args(&json);
    assert_eq!(a[0], Json::Null);
    assert_eq!(a[1], Json::Bool(true));
    assert_eq!(a[2], 7);
    assert_eq!(a[3], 1.5);
    assert_eq!(a[4]["sym"], "sym");
    assert_eq!(a[5]["str"], "text");
    assert_eq!(a[6]["list"][1], 2);
    assert_eq!(a[6]["tail"], 3);
    assert_eq!(a[7]["vector"][1]["sym"], "b");
    assert_eq!(a[7]["len"], 2);
    assert_eq!(a[8]["type"], "buffer");
}

#[test]
fn a_circular_list_and_a_deep_tree_stop_at_the_budget() {
    let json = snapshot(SHOW, "(let ((l (list 1 2))) (setcdr (cdr l) l) (i-show l))");
    assert_eq!(args(&json)[0]["elided"], true);
    assert_eq!(args(&json)[0]["list"].as_array().unwrap().len(), 63);
    let json = snapshot(SHOW, "(let ((tree 1)) (dotimes (_ 100000) (setq tree (list tree))) (i-show tree))");
    assert_eq!(args(&json)[0]["list"][0]["list"][0]["list"][0]["type"], "cons");
}

#[test]
fn many_arguments_and_a_long_stack_are_cut() {
    let json = snapshot(SHOW, "(apply #'i-show (make-list 1000 0))");
    assert_eq!(shown(&json)["argc"], 1000);
    assert_eq!(args(&json).as_array().unwrap().len(), 16);
    let json = snapshot("(progn (setq max-lisp-eval-depth 10000) (defun i-deep (n) (if (> n 0) (list (i-deep (1- n))) (romacs--inspect-frames))))",
                        "(let ((value (i-deep 1000))) (while (consp value) (setq value (car value))) value)");
    assert_eq!(json["shown"], 256);
    assert!(json["depth"].as_u64().unwrap() > 1000);
}

#[test]
fn a_huge_symbol_name_is_cut_and_the_snapshot_stays_in_bounds() {
    let json = snapshot(SHOW, "(let ((huge (intern (make-string 200000 ?x)))) (apply #'i-show (make-list 16 (make-list 60 huge))))");
    let symbol = &args(&json)[0]["list"][0];
    assert_eq!(symbol["sym"].as_str().unwrap().len(), 80);
    assert_eq!(symbol["cut"], true);
    assert!(json.to_string().len() < 200_000, "{}", json.to_string().len());
    let json = snapshot("(defalias (intern (make-string 200000 ?y)) (lambda () (romacs--inspect-frames)))",
                        "(funcall (intern (make-string 200000 ?y)))");
    assert!(json.to_string().len() < 200_000, "{}", json.to_string().len());
}

#[test]
fn a_circular_list_crossing_the_byte_limit_ends() {
    let json = snapshot(SHOW, "(let ((l (list (intern (make-string 2000 ?x))))) (setcdr l l) (apply #'i-show (make-list 16 l)))");
    assert!(json.to_string().len() < 200_000, "{}", json.to_string().len());
}

#[test]
fn secrets_stay_out() {
    let json = snapshot(SHOW, r#"(i-show "my Password is x" (make-string 5000 ?a) "plain")"#);
    assert_eq!(args(&json)[0]["redacted"], true);
    assert!(args(&json)[0].get("str").is_none());
    assert_eq!(args(&json)[1]["redacted"], true);
    assert_eq!(args(&json)[1]["len"], 5000);
    assert_eq!(args(&json)[2]["str"], "plain");
    let json = snapshot("(defun auth-source-i-show (&rest _) (romacs--inspect-frames))", r#"(auth-source-i-show "hunter2")"#);
    assert_eq!(args(&json), "redacted");
    assert!(!json.to_string().contains("hunter2"));
}
