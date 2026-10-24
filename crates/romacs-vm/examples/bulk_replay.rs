//! Diagnostic replay of a GNU-recorded bulk corpus: one JSON verdict line per
//! case on stdout, a per-primitive summary on stderr. Every case runs in a
//! FRESH production VM, so no case can leak state into another.
//!
//!     cargo run -p romacs-vm --example bulk_replay -- tools/primh/bulk/core.corpus.json
//!
//! The authoritative check is `tests/bulk_core.rs`; this only reports.
#[path = "../tests/support/bulk.rs"]
mod bulk;

use std::collections::BTreeMap;

fn main() {
    // Production's evaluator stack (romacs-vm-gui EVALUATOR_STACK_SIZE, R209).
    std::thread::Builder::new().stack_size(64 << 20).spawn(run).unwrap().join().unwrap();
}

fn run() {
    let path = std::env::args().nth(1).expect("usage: bulk_replay CORPUS.json");
    let ev: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    std::panic::set_hook(Box::new(|_| {}));
    let mut summary: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();
    let mut failed = false;
    for row in ev["cases"].as_array().unwrap() {
        let actual = bulk::run_case(row["source"].as_str().unwrap());
        let v = bulk::verdict(row, &actual);
        failed |= v != "PASS";
        *summary.entry(row["p"].as_str().unwrap().to_owned()).or_default().entry(v).or_default() += 1;
        println!(
            "{}",
            serde_json::json!({"id": row["id"], "p": row["p"], "family": row["family"],
                               "verdict": v, "expected": row["expected"],
                               "variant": row["gnu_regime_variant"], "actual": actual,
                               "source": row["source"]})
        );
    }
    for (p, counts) in &summary {
        eprintln!("{p:12} {}", serde_json::to_string(counts).unwrap());
    }
    std::process::exit(i32::from(failed));
}
