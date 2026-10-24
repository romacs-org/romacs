//! INITIAL terminal contracts recorded from the pinned GNU batch oracle.
//! No terminal emulator, display server, or process-specific object IDs.
use romacs_vm::Vm;
use serde_json::Value;
use std::collections::BTreeSet;

fn replay(name: &str, spec_bytes: &[u8], recorded: &str) {
    use sha2::{Digest, Sha256};
    let spec: Value = serde_json::from_slice(spec_bytes).unwrap();
    let recorded: Value = serde_json::from_str(recorded).unwrap();
    let hash: String = Sha256::digest(spec_bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(recorded["spec_sha256"], hash, "stale {name} spec");
    assert_eq!(recorded["primitive"], name);
    if spec.get("rust").is_some() {
        let manifest: Value =
            serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/primitives.json")))
                .unwrap();
        let declaration = manifest
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .expect("frame declaration missing");
        assert_eq!(
            spec["arity"],
            serde_json::json!([declaration["min"], declaration["max"]])
        );
        assert_eq!(spec["rust"], declaration["rust"], "stale Rust coordinates");
        assert_eq!(spec["gnu_def"], declaration["gnu"], "stale GNU coordinates");
        assert_eq!(
            spec["effect"].as_str().unwrap(),
            declaration["effect"].as_str().unwrap().to_lowercase()
        );
    }
    let evidence: Value =
        serde_json::from_str(include_str!("../tools/primh/frame-replay.json")).unwrap();
    assert_eq!(
        evidence["gnu_binary_sha256"],
        "a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8"
    );
    let rows = evidence["cases"].as_array().unwrap();
    let stable: Vec<_> = recorded["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| {
            let expected = r["expected"].as_str().unwrap();
            r["family"] != "printer"
                && !["#<frame ", "#<window ", "#<terminal "]
                    .iter()
                    .any(|s| expected.contains(s))
        })
        .collect();
    let selected: Vec<_> = rows.iter().filter(|r| r["primitive"] == name).collect();
    assert_eq!(
        selected.len(),
        stable.len(),
        "missing stable replay rows for {name}"
    );
    for (row, original) in selected.iter().zip(stable) {
        for field in ["id", "family", "source", "expected", "gnu_regime_variant"] {
            assert_eq!(
                row[field], original[field],
                "replay evidence drift: {name} {field}"
            );
        }
    }
    let mut ids = BTreeSet::new();
    let mut failures = Vec::new();
    for row in rows.iter().filter(|r| r["primitive"] == name) {
        let id = row["id"].as_str().unwrap();
        assert!(ids.insert(id), "duplicate replay case {id}");
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let src = row["source"].as_str().unwrap();
        let actual = vm
            .eval_source(src)
            .and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
            .unwrap_or_else(|e| format!("VM-ERROR {e}"));
        let expected = row["expected"].as_str().unwrap();
        // Compare compiled regimes only; GNU's second (interpreted) run can
        // observe state left by its preceding compiled run.
        if actual != expected {
            failures.push(format!("{id}: {src}\n GNU: {expected}\n VM2: {actual}"));
        }
    }
    assert!(!ids.is_empty(), "missing frame replay cases for {name}");
    assert!(
        failures.is_empty(),
        "{name}: {} GNU contract failures\n{}",
        failures.len(),
        failures.join("\n")
    );
}

macro_rules! contract {
    ($test:ident, $name:literal) => {
        #[test]
        fn $test() {
            replay(
                $name,
                include_bytes!(concat!("../tools/primh/frame-specs/", $name, ".json")),
                include_str!(concat!("../tools/primh/frame-evidence/", $name, ".json")),
            );
        }
    };
}
contract!(framep_gnu_contract, "framep");
contract!(frame_list_gnu_contract, "frame-list");
contract!(frame_terminal_gnu_contract, "frame-terminal");
contract!(terminal_live_p_gnu_contract, "terminal-live-p");
contract!(
    last_nonminibuffer_frame_gnu_contract,
    "last-nonminibuffer-frame"
);
contract!(terminal_list_gnu_contract, "terminal-list");
contract!(tty_top_frame_initial_is_not_a_tty, "tty-top-frame");
contract!(window_list_gnu_contract, "window-list");
contract!(get_buffer_window_gnu_contract, "get-buffer-window");
contract!(
    window_combination_limit_gnu_contract,
    "window-combination-limit"
);

#[test]
fn registered_frame_terminal_identity_survives_moving_collection() {
    let mut vm = Vm::new();
    vm.eval_source("(setq saved-frame (selected-frame) saved-terminal (frame-terminal) saved-frames (frame-list) saved-terminals (terminal-list))").unwrap();
    vm.collect_garbage();
    vm.collect_garbage();
    let actual = vm.eval_source("(list (eq saved-frame (selected-frame)) (eq saved-terminal (frame-terminal)) (eq (car saved-frames) saved-frame) (eq (car saved-terminals) saved-terminal) (framep saved-frame) (terminal-live-p saved-terminal) (eq (window-frame (selected-window)) saved-frame) (tty-top-frame saved-terminal))").unwrap().display().unwrap();
    assert_eq!(
        actual, "(t t t t t t t nil)",
        "registered frame/terminal identity and window backlink after moving GC"
    );
}
