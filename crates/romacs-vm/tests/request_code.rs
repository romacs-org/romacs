use romacs_vm::{Form, Vm};
#[path = "support/own_test.rs"]
mod own_test;

fn isolated(name: &str) -> bool {
    if std::env::var("VM2_GC_PROBE_CHILD").as_deref() == Ok(name) {
        return true;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), name), "--nocapture"])
        .env("VM2_GC_PROBE_CHILD", name)
        .status()
        .unwrap();
    assert!(status.success(), "GC probe {name} failed: {status}");
    false
}

fn src(vm: &mut Vm, s: &str) -> String {
    match vm.eval_source(s) {
        Ok(v) => v.display().unwrap_or_else(|e| format!("DISPLAYERR:{e}")),
        Err(e) => format!("ERR:{e}"),
    }
}
fn form(vm: &mut Vm, j: &str) -> String {
    let f = Form::from_json(serde_json::from_str(j).unwrap()).unwrap();
    match vm.eval_form(&f) {
        Ok(v) => v.display().unwrap_or_else(|e| format!("DISPLAYERR:{e}")),
        Err(e) => format!("ERR:{e}"),
    }
}
fn churn(vm: &mut Vm, n: usize, salt: usize) {
    for i in 0..n {
        let k = i * 7 + salt;
        let r = if i % 2 == 0 {
            form(
                vm,
                &format!(
                    r#"["funcall",["lambda",["y"],["+","y",{k},["length",["list","y","y"]]]],1]"#
                ),
            )
        } else {
            src(
                vm,
                &format!("(let ((f (lambda (z) (list z {k})))) (car (cdr (funcall f 0))))"),
            )
        };
        let want = if i % 2 == 0 {
            (k + 3).to_string()
        } else {
            k.to_string()
        };
        assert_eq!(r, want, "churn {i}");
        if i % 97 == 0 {
            vm.clear_source_cache();
            vm.collect_garbage();
        }
    }
    vm.clear_source_cache();
    vm.collect_garbage();
}
fn fresh() -> Vm {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm
}

#[test]
fn prepared_handles_survive_and_track_redefinition() {
    if !isolated("prepared_handles_survive_and_track_redefinition") {
        return;
    }
    let mut vm = fresh();
    src(&mut vm, "(defun base (x) (* x 3))");
    let h = vm.prepare_source("(lambda (x) (+ 1 (base x)))").unwrap();
    churn(&mut vm, 3000, 12);
    let v = vm
        .invoke_prepared(h, &[romacs_vm::Fixed::integer(2).unwrap().into()])
        .unwrap();
    assert_eq!(v.display().unwrap(), "7");
    src(&mut vm, "(defun base (x) (* x 10))");
    churn(&mut vm, 3000, 13);
    let v = vm
        .invoke_prepared(h, &[romacs_vm::Fixed::integer(2).unwrap().into()])
        .unwrap();
    assert_eq!(v.display().unwrap(), "21");
}

#[test]
fn warm_funcall_site_vs_reused_function_ids() {
    if !isolated("warm_funcall_site_vs_reused_function_ids") {
        return;
    }
    let mut vm = fresh();
    src(&mut vm, "(defun call-it (f) (funcall f 5))");
    for round in 0..400 {
        let r = form(
            &mut vm,
            &format!(r#"["call-it",["lambda",["x"],["+","x",{round}]]]"#),
        );
        assert_eq!(r, (5 + round).to_string(), "round {round}");
        for j in 0..5 {
            let r = form(
                &mut vm,
                &format!(r#"["funcall",["lambda",["a","b"],["list","a","b",{j}]],1,2]"#),
            );
            assert_eq!(r, format!("(1 2 {j})"));
        }
        vm.clear_source_cache();
        vm.collect_garbage();
    }
    for round in 0..400 {
        let r = src(
            &mut vm,
            &format!("(call-it (let ((k (list {round}))) (lambda (x) (- x (car k)))))"),
        );
        assert_eq!(r, (5 - round as i64).to_string(), "src round {round}");
        vm.clear_source_cache();
        vm.collect_garbage();
    }
}
