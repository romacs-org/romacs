//! Measurement adapter only: a fresh VM per process, no subject changes.
//!
//! Two modes, and every report says which one produced it (R210):
//!
//! * COLD (default): `romacs-vm-census FILE LOADPATH...`. Nothing is
//!   preloaded; the raw-capability diagnostic the census has always measured.
//! * WARM: `romacs-vm-census --preload LINK ... -- FILE LOADPATH...`. VM2's
//!   built-in Lisp prelude is installed, then each bootstrap-chain LINK (GNU
//!   `lisp/loadup.el` order) is loaded, stopping at the first link that fails,
//!   exactly as far as the chain reaches. Only then is FILE measured. The
//!   chain's steps and counters are cut from the report so they can never be
//!   counted as the target's progress, even when FILE is itself a chain link.
use romacs_vm::Vm;

fn chain_entry(link: &std::path::Path, report: &serde_json::Value, from: usize, result: &Result<(), String>) -> serde_json::Value {
    let name = link.to_string_lossy();
    let steps = report["steps"].as_array().map(|s| &s[from.min(s.len())..]).unwrap_or(&[]);
    let own: Vec<_> = steps.iter().filter(|s| s["file"] == name.as_ref()).collect();
    let mut entry = serde_json::json!({
        "file": name,
        "loaded": result.is_ok(),
        "forms_completed": own.iter().filter(|s| s["status"] == "ok").count(),
    });
    if let Err(error) = result {
        entry["error"] = error.chars().take(400).collect::<String>().into();
        if let Some(frontier) = own.iter().find(|s| s["status"] != "ok") {
            entry["frontier"] = serde_json::json!({
                "ordinal": frontier["ordinal"],
                "line": frontier["line"],
                "error": frontier.get("error").cloned().unwrap_or(serde_json::Value::Null),
                "source": frontier["source"].as_str().unwrap_or("").chars().take(200).collect::<String>(),
            });
        }
    }
    entry
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut preload = Vec::new();
    let mut eager_target = false;
    loop {
        match args.first().map(String::as_str) {
            Some("--preload") if args.len() > 1 => {
                args.remove(0);
                preload.push((false, args.remove(0)));
            }
            Some("--preload-form") if args.len() > 1 => {
                args.remove(0);
                preload.push((true, args.remove(0)));
            }
            Some("--eager-target") => {
                args.remove(0);
                eager_target = true;
            }
            _ => break,
        }
    }
    if args.first().is_some_and(|a| a == "--") {
        args.remove(0);
    }
    let warm = !preload.is_empty();
    let file = std::fs::canonicalize(&args[0]).unwrap();
    let mut vm = Vm::new();
    // GNU's census oracle runs `--batch`, as the ERT runner's regime does.
    vm.eval_source("(setq noninteractive t)").unwrap();
    let paths = args[1..].iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
    vm.set_load_path(&paths).unwrap();
    let mut chain = Vec::new();
    let mut chain_panicked = false;
    if warm {
        // The prelude is VM2's Lisp implementation of GNU C primitives
        // (mapcar). GNU has those before loadup.el runs, so it is runtime.
        let prelude = vm.catching(|vm| vm.install_lisp_prelude().map(|_| ()));
        if !matches!(prelude, Ok(Ok(_))) {
            chain_panicked = prelude.is_err();
            chain.push(serde_json::json!({"file": "<vm2 lisp prelude>", "loaded": false, "forms_completed": 0}));
        } else {
            for (is_form, input) in &preload {
                let path = std::path::PathBuf::from(input);
                let link = if *is_form { path } else { std::fs::canonicalize(&path).unwrap_or(path) };
                let from = vm.load_report()["steps"].as_array().map_or(0, Vec::len);
                let result = match vm.catching(|vm| {
                    if *is_form { vm.eval_source(input).map(|_| ()) }
                    else { vm.load_file(&link).map(|_| ()) }
                }) {
                    Ok(Ok(_)) => Ok(()),
                    Ok(Err(error)) => Err(error),
                    Err(_) => {
                        chain_panicked = true;
                        Err("panic while loading bootstrap chain".to_owned())
                    }
                };
                let failed = result.is_err();
                let mut entry = chain_entry(&link, &vm.load_report(), from, &result);
                if *is_form {
                    entry["file"] = "<preload form>".into();
                    entry["form"] = input.clone().into();
                    entry["kind"] = "form".into();
                    let report = vm.load_report();
                    let steps = report["steps"].as_array().map(|s| &s[from..]).unwrap_or(&[]);
                    entry["forms_completed"] = steps.iter().filter(|s| s["status"] == "ok").count().into();
                    let mut files = Vec::new();
                    for step in steps {
                        if let Some(file) = step["file"].as_str() {
                            if !files.contains(&file) { files.push(file); }
                        }
                    }
                    entry["executed_loads"] = files.iter().map(|file| {
                        let own: Vec<_> = steps.iter().filter(|s| s["file"] == *file).collect();
                        serde_json::json!({"file": file,
                            "forms_completed": own.iter().filter(|s| s["status"] == "ok").count(),
                            "failed": own.iter().any(|s| s["status"] != "ok")})
                    }).collect::<Vec<_>>().into();
                    if let Some(step) = steps.iter().find(|s| s["status"] != "ok") {
                        entry["frontier"] = serde_json::json!({"file": step["file"],
                            "ordinal": step["ordinal"], "line": step["line"], "error": step["error"]});
                    }
                }
                chain.push(entry);
                if failed {
                    break;
                }
            }
        }
    }
    if eager_target && !chain_panicked {
        vm.eval_source("(fset 'internal-macroexpand-for-load (lambda (form &rest ignored) form))")
            .expect("install census-only eager counterfactual identity after bootstrap");
    }
    let chain_report = vm.load_report();
    let chain_steps = chain_report["steps"].as_array().map_or(0, Vec::len);
    let chain_forms = chain_report["completed_forms"].as_u64().unwrap_or(0);
    let chain_files = chain_report["completed_files"].as_u64().unwrap_or(0);
    // Use the real loader directly so reader/compile error text survives.
    // A VM that panicked during the chain is not measured at all.
    let result = if chain_panicked {
        Err(Box::new("bootstrap chain panicked") as Box<dyn std::any::Any + Send>)
    } else {
        vm.catching(|vm| vm.load_file(&file).map(|_| ()))
    };
    let mut report = vm.load_report();
    if warm {
        if let Some(steps) = report["steps"].as_array_mut() {
            steps.drain(..chain_steps.min(steps.len()));
        }
        report["completed_forms"] = report["completed_forms"].as_u64().unwrap_or(0).saturating_sub(chain_forms).into();
        report["completed_files"] = report["completed_files"].as_u64().unwrap_or(0).saturating_sub(chain_files).into();
        report["chain"] = serde_json::Value::Array(chain);
    }
    report["mode"] = if warm { "warm" } else { "cold" }.into();
    if eager_target {
        report["eager_target"] = true.into();
        report["measurement_regime"] = "eager_counterfactual".into();
    }
    report["file"] = file.to_string_lossy().into_owned().into();
    match result {
        Ok(Ok(_)) => { report["ok"] = true.into(); }
        Ok(Err(error)) => { report["ok"] = false.into(); report["error"] = error.into(); }
        Err(_) => { report["ok"] = false.into(); report["panic"] = true.into(); }
    }
    use std::io::Write;
    report["measurement"] = true.into();
    println!("{}", serde_json::to_string(&report).unwrap());
    std::io::stdout().flush().unwrap();
    // Diagnostic replay occurs AFTER the measured verdict and counters freeze.
    // No replay result contributes to loaded/forms_evaluated. Preserve the
    // structured signal before the loader wraps its error as an untyped String.
    let cause = report["steps"].as_array().and_then(|steps| {
        let first = steps.iter().position(|s| s["file"] == report["file"] && s["status"] != "ok")?;
        let mut cause = &steps[first];
        for step in &steps[first + 1..] {
            let prefix = format!("{}: line {} form {}:", step["file"].as_str().unwrap_or(""), step["line"], step["ordinal"]);
            if step["status"] != "ok" && cause["error"].as_str().unwrap_or("").contains(&prefix) { cause = step; }
        }
        cause["source"].as_str().map(str::to_owned)
    });
    if report["ok"] == false {
        if let Some(source) = cause {
            let replay = format!("(condition-case census--replay {source} (error census--replay))");
            let replayed = vm.catching(|vm| vm.eval_source(&replay).and_then(|v| v.display()));
            if let Ok(replay_result) = replayed {
                let text = match replay_result { Ok(text) => text, Err(error) => error };
                report["diagnostic_condition"] = text.clone().into();
                if text == "(void-function)" || text == "void or non-function" {
                    // Candidate-only call attribution. These sentinels run only
                    // after the verdict; fboundp-sensitive code can diverge, so
                    // Python never labels this a confirmed missing symbol.
                    // The candidates are the symbols of every file a step was
                    // read from (a completed step keeps no text of its own).
                    let files: std::collections::BTreeSet<&str> = report["steps"].as_array().unwrap().iter()
                        .filter_map(|step| step["file"].as_str()).collect();
                    let mut tokens = std::collections::BTreeSet::new();
                    for file in files {
                        // Lossy: the scan wants ASCII symbol names, whatever the file's coding.
                        let text = String::from_utf8_lossy(&romacs_vm::host::read_file(file).unwrap_or_default()).into_owned();
                        for token in text.split(|c: char| !c.is_ascii_alphanumeric() && !"-+*/<>=!?_:".contains(c)) {
                            if !token.is_empty() && token.as_bytes()[0].is_ascii_alphabetic() { tokens.insert(token.to_owned()); }
                        }
                    }
                    for token in tokens {
                        let name = serde_json::to_string(&token).unwrap();
                        let marker = serde_json::to_string(&format!("CENSUS-MISSING-FUNCTION:{token}")).unwrap();
                        let stub = format!("(let ((s (intern {name}))) (unless (fboundp s) (fset s (lambda (&rest census--args) (error {marker})))))");
                        let _ = vm.eval_source(&stub);
                    }
                    if let Ok(result) = vm.catching(|vm| vm.eval_source(&replay).and_then(|v| v.display())) {
                        report["candidate_replay"] = result.unwrap_or_else(|e| e).into();
                    }
                }
            }
        }
    }
    println!("{}", serde_json::to_string(&report).unwrap());
}
