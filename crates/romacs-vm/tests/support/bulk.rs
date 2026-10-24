// Shared by `tests/bulk_core.rs` and `examples/bulk_replay.rs`: replay a
// GNU-recorded bulk corpus (`tools/primh/bulk.py record`) through the
// production VM. Same conventions as primh's `replay.rs.in`: the primh WRAP
// source, a fresh VM per row at collection floor 1, either GNU regime counts as
// parity, and the same verdict classes. A row's expectation is GNU's output and
// nothing else; this module never computes an expected value.
#![allow(dead_code)]
use romacs_vm::Vm;
use serde_json::Value;

/// Evaluate SOURCE in a fresh production VM: built-in prelude installed (as
/// the GUI and the census do), collection at every safepoint.
pub fn run_case(source: &str) -> String {
    let src = source.to_owned();
    let outcome = std::panic::catch_unwind(move || {
        let mut vm = Vm::new();
        if let Err(e) = vm.install_lisp_prelude() {
            return format!("VM-ERROR prelude {e}");
        }
        vm.set_collection_floor(1);
        match vm.eval_source(&src) {
            Ok(v) => v
                .display()
                .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
                .unwrap_or_else(|e| format!("VM-ERROR {e}")),
            Err(e) => format!("VM-ERROR {e}"),
        }
    });
    outcome.unwrap_or_else(|_| "VM-CRASH".to_owned())
}

/// primh's classification (`replay.rs.in` `classify`), unchanged.
pub fn classify(expected: &str, actual: &str) -> &'static str {
    if expected == actual {
        return "PASS";
    }
    if actual == "VM-CRASH" {
        return "VM-CRASH";
    }
    if actual.starts_with("VM-ERROR") {
        return if actual.contains("print") { "PRINTER" } else { "VM-INTERNAL" };
    }
    if actual.starts_with("(sig (void-function") {
        return "MISSING";
    }
    let e_sig = expected.starts_with("(sig ");
    let a_sig = actual.starts_with("(sig ");
    if e_sig && a_sig {
        let condition = |s: &str| s[6..].split(' ').next().unwrap().trim_end_matches(')').to_owned();
        return if condition(expected) == condition(actual) { "DATA" } else { "CONDITION" };
    }
    if e_sig != a_sig {
        return "SIGNAL-VS-VALUE";
    }
    if expected.contains("#<") || expected.contains("#[") || actual.contains("#<") {
        return "PRINTER";
    }
    "VALUE"
}

/// PASS when VM2 equals GNU's byte-compiled answer or its interpreted one.
pub fn verdict(row: &Value, actual: &str) -> &'static str {
    let expected = row["expected"].as_str().unwrap();
    if actual == expected || row["gnu_regime_variant"].as_str() == Some(actual) {
        "PASS"
    } else {
        classify(expected, actual)
    }
}
