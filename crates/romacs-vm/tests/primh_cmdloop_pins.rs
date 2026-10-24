//! A primh pin stores only the divergence CLASS, so a pinned row keeps passing
//! when VM2's wrong answer changes into a different wrong answer of the same
//! class. This locks every pinned command-loop row and the sixteen additional
//! reviewed opaque-closure observation rows to their exact VM2 outcomes,
//! together with the source and GNU answer reviewed for each. In particular,
//! VM2-CLOSEQ1/VM2-FR5-1 sequence refusals remain semantic divergences; a
//! printable opaque placeholder is not closure-slot compatibility.
//!
//! Re-lock deliberately, after reviewing each changed outcome:
//!   ROMACS_ACCEPT_PINS=1 cargo test -p romacs-vm --test primh_cmdloop_pins
//! which rewrites `primh_cmdloop_pins.json` and then fails, so accepting can
//! never be what makes the suite green.
use romacs_vm::Vm;
use serde_json::{json, Map, Value};

const EVIDENCE: [(&str, &str); 5] = [
    ("run-hook-with-args", include_str!("../tools/primh/evidence/run-hook-with-args.json")),
    ("prefix-numeric-value", include_str!("../tools/primh/evidence/prefix-numeric-value.json")),
    ("funcall-interactively", include_str!("../tools/primh/evidence/funcall-interactively.json")),
    ("interactive-form", include_str!("../tools/primh/evidence/interactive-form.json")),
    ("call-interactively", include_str!("../tools/primh/evidence/call-interactively.json")),
];
const LOCKS: &str = include_str!("primh_cmdloop_pins.json");
const CLOSURE_EVIDENCE: [(&str, &str); 7] = [
    ("reverse", include_str!("../tools/primh/evidence/reverse.json")),
    ("vconcat", include_str!("../tools/primh/evidence/vconcat.json")),
    ("mapc", include_str!("../tools/primh/evidence/mapc.json")),
    ("mapcan", include_str!("../tools/primh/evidence/mapcan.json")),
    ("prin1", include_str!("../tools/primh/evidence/prin1.json")),
    ("princ", include_str!("../tools/primh/evidence/princ.json")),
    ("print", include_str!("../tools/primh/evidence/print.json")),
];
// Only these already-pinned rows are in the adjacent review, not every pin of
// those seven corpora. The other two closure rows are already in EVIDENCE.
const CLOSURE_IDS: [&str; 16] = [
    "reverse/0049", "vconcat/0043", "mapc/0081",
    "mapcan/0025", "mapcan/0052", "mapcan/0079", "mapcan/0106",
    "mapcan/0133", "mapcan/0160", "mapcan/0187", "mapcan/0214", "mapcan/0241",
    "prin1/0036", "prin1/0113", "princ/0036", "print/0036",
];

fn reviewed_rows(eval: &dyn Fn(&str) -> String) -> Vec<(String, Value, String)> {
    let mut rows = pinned_rows(&EVIDENCE, eval);
    let closure_rows = pinned_rows_scoped(&CLOSURE_EVIDENCE, eval, Some(&CLOSURE_IDS));
    let ids: std::collections::BTreeSet<_> = closure_rows.iter().map(|(id, _, _)| id.as_str()).collect();
    assert_eq!(ids, CLOSURE_IDS.into_iter().collect(), "reviewed closure pin inventory changed");
    rows.extend(closure_rows);
    rows
}

// The same observation as tests/primh_cmdloop.rs: a fresh VM per row,
// collecting at every safepoint.
fn observe(src: &str) -> String {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    match vm.eval_source(src) {
        Ok(v) => v.display().map(|s| serde_json::from_str::<String>(&s).unwrap_or(s)).unwrap_or_else(|e| format!("VM-ERROR {e}")),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

/// Every pinned row of EVIDENCE with its current outcome.
fn pinned_rows(evidence: &[(&str, &str)], eval: &dyn Fn(&str) -> String) -> Vec<(String, Value, String)> {
    pinned_rows_scoped(evidence, eval, None)
}

fn pinned_rows_scoped(evidence: &[(&str, &str)], eval: &dyn Fn(&str) -> String, scope: Option<&[&str]>) -> Vec<(String, Value, String)> {
    let mut out = Vec::new();
    for (name, text) in evidence {
        let ev: Value = serde_json::from_str(text).unwrap();
        assert_eq!(ev["primitive"], *name);
        let pinned = ev["pinned"].as_object().cloned().unwrap_or_default();
        for row in ev["cases"].as_array().unwrap() {
            let id = row["id"].as_str().unwrap();
            if scope.is_some_and(|ids| !ids.contains(&id)) { continue; }
            if let Some(class) = pinned.get(id) {
                let actual = eval(row["source"].as_str().unwrap());
                out.push((id.to_owned(), json!({"class": class, "source": row["source"], "expected": row["expected"]}), actual));
            }
        }
    }
    out
}

fn drift(rows: &[(String, Value, String)], locks: &Map<String, Value>) -> Vec<String> {
    let mut bad = Vec::new();
    for (id, row, actual) in rows {
        match locks.get(id) {
            None => bad.push(format!("UNLOCKED PIN ({}): {id}\n  VM2 {actual}", row["class"])),
            Some(lock) if lock["source"] != row["source"] || lock["expected"] != row["expected"] || lock["class"] != row["class"] => {
                bad.push(format!("STALE LOCK: {id} (its row, GNU answer or class changed; re-review)"))
            }
            Some(lock) if lock["accepted"].as_str() != Some(actual.as_str()) => bad.push(format!(
                "DRIFT ({}): {id}\n  GNU      {}\n  accepted {}\n  now      {actual}",
                row["class"], row["expected"], lock["accepted"]
            )),
            Some(_) => {}
        }
    }
    for id in locks.keys() {
        if !rows.iter().any(|(r, _, _)| r == id) {
            bad.push(format!("ORPHAN LOCK: {id} (no longer pinned)"));
        }
    }
    bad
}

#[test]
fn pinned_rows_keep_their_exact_vm2_outcome() {
    let rows = reviewed_rows(&observe);
    if std::env::var_os("ROMACS_ACCEPT_PINS").is_some() {
        let locks: Map<String, Value> = rows.iter().map(|(id, row, actual)| {
            let mut lock = row.clone();
            lock["accepted"] = json!(actual);
            (id.clone(), lock)
        }).collect();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/primh_cmdloop_pins.json");
        std::fs::write(path, serde_json::to_string_pretty(&locks).unwrap() + "\n").unwrap();
        panic!("wrote {} locks to {path}; review the diff, then re-run without ROMACS_ACCEPT_PINS", locks.len());
    }
    let locks: Map<String, Value> = serde_json::from_str(LOCKS).unwrap();
    let bad = drift(&rows, &locks);
    assert!(bad.is_empty(), "{} pinned command-loop rows:\n{}", bad.len(), bad.join("\n"));
}

// r64: the class-only ratchet accepts this; the lock must not.
#[test]
fn a_same_class_change_of_a_pinned_answer_is_drift() {
    let ev = json!({"primitive": "p", "pinned": {"p/0001": "VALUE"},
                    "cases": [{"id": "p/0001", "source": "(p)", "expected": "(ok 1)"}]}).to_string();
    let locks: Map<String, Value> = serde_json::from_value(json!({"p/0001":
        {"class": "VALUE", "source": "(p)", "expected": "(ok 1)", "accepted": "(ok 9)"}})).unwrap();
    assert!(drift(&pinned_rows(&[("p", &ev)], &|_| "(ok 9)".into()), &locks).is_empty());
    let changed = drift(&pinned_rows(&[("p", &ev)], &|_| "(ok 123)".into()), &locks);
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert!(changed[0].starts_with("DRIFT (\"VALUE\"): p/0001"), "{changed:?}");
    let unlocked = drift(&pinned_rows(&[("p", &ev)], &|_| "(ok 9)".into()), &Map::new());
    assert!(unlocked[0].starts_with("UNLOCKED PIN"), "{unlocked:?}");
}
