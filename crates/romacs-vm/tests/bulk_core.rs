//! Bulk differential replay of the `core` group: car cdr cons list eq equal
//! null stringp length memq nth append substring nreverse mapcar.
//!
//! Every expectation is GNU's recorded output (`tools/primh/bulk.py record`,
//! GNU 32.0.50 through the unchanged primh `gnu_driver.el`); this file and the
//! spec never state an answer. Each row runs in a FRESH production VM (prelude
//! installed, collection at every safepoint). A difference is either fixed or
//! pinned in `bulk/core.ledger.json` to a row of `/DIVERGENCES.md`; a pin that
//! stops differing, changes class, names an unknown case or names a missing
//! ledger row fails, exactly as primh's pins do.
#[path = "support/bulk.rs"]
mod bulk;

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const CORPUS: &str = include_str!("../tools/primh/bulk/core.corpus.json");
const LEDGER: &str = include_str!("../tools/primh/bulk/core.ledger.json");
/// Fewer successful GNU answers than this for one primitive is a corpus that
/// mostly proves the primitive signals, which proves little (bulk.py report).
const MIN_OK_PER_PRIMITIVE: usize = 20;

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn validate(ev: &Value) {
    assert_eq!(ev["protocol"], "bulk-v1");
    assert_eq!(ev["generator_sha256"], hash(include_bytes!("../tools/primh/bulk.py")), "bulk.py changed; re-record");
    assert_eq!(ev["driver_sha256"], hash(include_bytes!("../tools/primh/gnu_driver.el")), "driver changed; re-record");
    assert_eq!(ev["spec_sha256"], hash(include_bytes!("../tools/primh/bulk/core.spec.json")), "spec changed; re-record");
    let spec: Value = serde_json::from_slice(include_bytes!("../tools/primh/bulk/core.spec.json")).unwrap();
    let declared: BTreeSet<&str> = spec["primitives"].as_object().unwrap().keys().map(String::as_str).collect();
    for subr in ev["gnu"]["subrs"].as_array().unwrap() {
        assert_eq!(subr[1], "subr", "{}: GNU defines it in Lisp; not a C primitive", subr[0]);
    }
    let rows = ev["cases"].as_array().unwrap();
    let ids: BTreeSet<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), rows.len(), "duplicate case IDs");
    let mut ok: BTreeMap<&str, usize> = declared.iter().map(|p| (*p, 0)).collect();
    for row in rows {
        let expected = row["expected"].as_str().unwrap();
        assert!(!expected.starts_with("HARNESS-ERROR"), "{}: unrecorded GNU answer", row["id"]);
        if let Some(n) = ok.get_mut(row["p"].as_str().unwrap()) {
            *n += usize::from(expected.starts_with("(ok "));
        }
    }
    for (p, n) in ok {
        assert!(n >= MIN_OK_PER_PRIMITIVE, "{p}: only {n} successful GNU cases; the corpus is vacuous for it");
    }
}

/// The divergence ledger, one file per divergence, read as one text.
fn divergences() -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ledger/divergences");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    paths.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect::<Vec<_>>().join("\n")
}

fn ledger_rows() -> BTreeSet<String> {
    divergences()
        .lines()
        .filter_map(|l| l.strip_prefix("| "))
        .filter_map(|l| l.split(' ').next())
        .map(str::to_owned)
        .collect()
}

/// Replay ROWS against PINS; return every violation.
fn replay(ev: &Value, pins: &serde_json::Map<String, Value>) -> Vec<String> {
    let rows = ev["cases"].as_array().unwrap();
    let ids: BTreeSet<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    let known = ledger_rows();
    let mut bad = Vec::new();
    for (id, pin) in pins {
        if !ids.contains(id.as_str()) {
            bad.push(format!("UNKNOWN PIN: {id}"));
        }
        let row = pin["divergence"].as_str().unwrap_or("");
        if !known.contains(row) {
            bad.push(format!("PIN WITHOUT LEDGER ROW: {id} names {row:?}, absent from ledger/divergences"));
        }
    }
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let chunk = rows.len().div_ceil(threads);
    // Production evaluates on a 64 MiB stack (romacs-vm-gui EVALUATOR_STACK_SIZE,
    // R209); a default 2 MiB test thread would abort on cases production survives.
    let actuals: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = rows
            .chunks(chunk)
            .map(|part| {
                std::thread::Builder::new()
                    .stack_size(64 << 20)
                    .spawn_scoped(s, move || part.iter().map(|r| bulk::run_case(r["source"].as_str().unwrap())).collect::<Vec<_>>())
                    .unwrap()
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    for (row, actual) in rows.iter().zip(&actuals) {
        let id = row["id"].as_str().unwrap();
        let verdict = bulk::verdict(row, actual);
        let pin = pins.get(id).and_then(|p| p["class"].as_str());
        match (verdict == "PASS", pin) {
            (true, Some(c)) => bad.push(format!("STALE PIN ({c}): {id}")),
            (false, None) => bad.push(format!(
                "NEW DIVERGENCE ({verdict}): {id}: {}\n  GNU {}\n  VM2 {actual}",
                row["source"].as_str().unwrap(),
                row["expected"].as_str().unwrap()
            )),
            (false, Some(c)) if c != verdict => bad.push(format!("CHANGED PIN ({c} -> {verdict}): {id}")),
            _ => {}
        }
    }
    bad
}

fn corpus() -> Value {
    serde_json::from_str(CORPUS).unwrap()
}

fn pins() -> serde_json::Map<String, Value> {
    let ledger: Value = serde_json::from_str(LEDGER).unwrap();
    ledger["pins"].as_object().cloned().unwrap_or_default()
}

#[test]
fn bulk_core_matches_gnu_case_for_case() {
    let ev = corpus();
    validate(&ev);
    let bad = replay(&ev, &pins());
    assert!(bad.is_empty(), "{} core-group violations:\n{}", bad.len(), bad.join("\n"));
}

/// primh's derived `nreverse` sweep (tools/primh/evidence/nreverse.json, from
/// `primh from-manifest` + `infer` + `record`). primh's own Rust replay binds
/// evidence to a `defsubr!` declaration, and `nreverse` stays on the legacy
/// enum (src/tails.rs header), so the same GNU-recorded rows are replayed here,
/// fresh VM per case, with primh's evidence hashes still enforced.
#[test]
fn primh_nreverse_evidence_matches_gnu_case_for_case() {
    let ev: Value = serde_json::from_str(include_str!("../tools/primh/evidence/nreverse.json")).unwrap();
    assert_eq!(ev["protocol"], "primh-v1");
    assert_eq!(ev["primitive"], "nreverse");
    assert_eq!(ev["spec_sha256"], hash(include_bytes!("../tools/primh/fns-core-specs/nreverse.json")), "spec changed; re-record");
    assert_eq!(ev["generator_sha256"], hash(include_bytes!("../tools/primh/primh.py")), "primh.py changed; re-record");
    assert_eq!(ev["driver_sha256"], hash(include_bytes!("../tools/primh/gnu_driver.el")), "driver changed; re-record");
    let rows = ev["cases"].as_array().unwrap();
    let ok = rows.iter().filter(|r| r["expected"].as_str().unwrap().starts_with("(ok ")).count();
    assert!(rows.len() >= 50 && ok >= MIN_OK_PER_PRIMITIVE, "{} rows, {ok} successful", rows.len());
    let mut pins = serde_json::Map::new();
    for (id, class) in ev["pinned"].as_object().unwrap() {
        assert_eq!(class, "PRINTER", "{id}: only printer limitations may be pinned here");
        pins.insert(id.clone(), serde_json::json!({"class": "PRINTER", "divergence": "VM2-EV2"}));
    }
    let bad = replay(&ev, &pins);
    assert!(bad.is_empty(), "{} nreverse violations:\n{}", bad.len(), bad.join("\n"));
}

// The harness itself must discriminate. Each mutation below changes exactly
// one thing about real recorded evidence and must be refused.

fn one_row(id: &str) -> Value {
    let mut ev = corpus();
    let row = ev["cases"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap().clone();
    ev["cases"] = serde_json::json!([row]);
    ev
}

#[test]
fn bulk_harness_refuses_a_changed_gnu_answer() {
    let mut ev = one_row("car/0000");
    assert!(replay(&ev, &Default::default()).is_empty(), "control row must pass");
    ev["cases"][0]["expected"] = serde_json::json!("(ok t)");
    let bad = replay(&ev, &Default::default());
    assert!(bad.len() == 1 && bad[0].starts_with("NEW DIVERGENCE"), "{bad:?}");
}

#[test]
fn bulk_harness_refuses_stale_unknown_and_unledgered_pins() {
    let ev = one_row("car/0000");
    let mut pins = serde_json::Map::new();
    pins.insert("car/0000".into(), serde_json::json!({"class": "VALUE", "divergence": "VM2-EV2"}));
    pins.insert("car/9999".into(), serde_json::json!({"class": "VALUE", "divergence": "VM2-EV2"}));
    pins.insert("cdr/0000".into(), serde_json::json!({"class": "VALUE", "divergence": "VM2-NO-SUCH-ROW"}));
    let bad = replay(&ev, &pins).join("\n");
    assert!(bad.contains("STALE PIN (VALUE): car/0000"), "{bad}");
    assert!(bad.contains("UNKNOWN PIN: car/9999"), "{bad}");
    assert!(bad.contains("PIN WITHOUT LEDGER ROW: cdr/0000"), "{bad}");
}

#[test]
fn bulk_harness_refuses_a_vacuous_corpus() {
    let mut ev = corpus();
    let kept: Vec<Value> = ev["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["p"] != "car" || !r["expected"].as_str().unwrap().starts_with("(ok "))
        .cloned()
        .collect();
    ev["cases"] = Value::Array(kept);
    let failure = std::panic::catch_unwind(|| validate(&ev)).expect_err("signal-only corpus accepted");
    let message = failure.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("car: only 0 successful GNU cases"), "{message}");
}
