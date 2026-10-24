//! An unsupported path must signal an error that names what is unsupported,
//! never an anonymous one: a caught condition is all a package sees.
use romacs_vm::Vm;

fn caught(src: &str) -> String {
    let mut vm = Vm::new();
    let form = format!("(prin1-to-string (condition-case e {src} (error e)))");
    let v = vm.eval_source(&form).expect("eval");
    let s = v.display().expect("display");
    serde_json::from_str::<String>(&s).unwrap_or(s)
}

// GNU stores a `(lambda ...)' list as the function (data.c Fdefalias); VM2
// has no list function descriptors yet (VM2-CALLINT1 (8), VM2-RUNHOOKS1).
#[test]
fn defalias_to_a_lambda_list_names_what_is_unsupported() {
    let got = caught("(defalias 'cl-j '(lambda (x) (interactive \"P\") x))");
    assert!(got.starts_with("(error \"") && got.contains("list function descriptor") && got.contains("VM2"), "{got}");
}
