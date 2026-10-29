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

