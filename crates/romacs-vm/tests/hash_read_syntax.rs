//! `#s(...)` read and print syntax against live-GNU answers.
//!
//! GNU reads `#s(hash-table ...)` through `hash_table_from_plist`
//! (src/lread.c:3153, dispatched at :3770-3780 from the `#s(` reader state at
//! :3852) and prints tables through `print_object`'s PVEC_HASH_TABLE arm
//! (src/print.c:2631-2675). `subr.el` form 497 (line 7701) is the round trip
//! `(hash-table-p (car (read-from-string (prin1-to-string (make-hash-table)))))`.
//!
//! Every expected answer is GNU 32.0.50's, recorded by
//! `evidence/hash-read-syntax/oracle.py` from `cases.json` into `gnu.json`;
//! no GNU binary is needed here. A case VM2 answers differently must be
//! listed in `DIVERGENT` with its ledger row and VM2's exact answer, and the
//! test fails if such a case starts matching GNU (a stale pin), so the
//! divergence set can only shrink.
use romacs_vm::Vm;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const GNU: &str = include_str!("../evidence/hash-read-syntax/gnu.json");
const CASES: &str = include_str!("../evidence/hash-read-syntax/cases.json");
const ORACLE: &str = include_str!("../evidence/hash-read-syntax/oracle.py");

/// (case id, ledger row, VM2's exact answer).
const DIVERGENT: &[(&str, &str, &str)] = &[
    // Completed numbered aliases are now checked against the GNU answer.
    // Hash-table limits/circles explicitly refuse; nested list print-level
    // now agrees with GNU and therefore has no divergence pin.
    ("p-print-length", "VM2-EV2", r#"(SIGNAL (error "hash-table print-length is not supported"))"#),
    ("p-print-length-0", "VM2-EV2", r#"(SIGNAL (error "hash-table print-length is not supported"))"#),
    ("p-circle-shared", "VM2-EV2", r#"(SIGNAL (error "hash-table print-circle is not supported"))"#),
    ("p-self-ref-circle", "VM2-EV2", r#"(SIGNAL (error "hash-table print-circle is not supported"))"#),
    ("p-self-ref-nocircle", "VM2-EV2", r#"(SIGNAL (error "print nesting limit exceeded"))"#),
    // The dot is read as GNU does; two pre-existing reader boundaries remain.
    // GNU's "expected )" after a dotted tail is VM2's internal message
    // (also pinned as read-from-string/0170).
    ("dot-two-after", "VM2-READ5", r#"(SIGNAL (invalid-read-syntax "dotted list requires one tail at byte 8"))"#),
    // NO-BREAK SPACE and VT are whitespace to GNU (lread.c:3566), not to
    // VM2's tokenizer, so the tail is the symbol "\u{a0}b" / "\u{b}b"
    // (also read-from-string/0164-0165).
    ("dot-before-nbsp", "VM2-READ2", "((a . \\\u{a0}b) . 7)"),

];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One fresh VM per case, collecting at every safepoint.
fn vm2_answer(prelude: &str, source: &str) -> String {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.eval_source(prelude).unwrap();
    match vm.eval_source(source) {
        Ok(value) => {
            let shown = value.display().unwrap();
            serde_json::from_str::<String>(&shown).unwrap_or(shown)
        }
        Err(error) => format!("VM2-ERROR {error}"),
    }
}

#[test]
fn hash_table_read_and_print_syntax_matches_gnu() {
    let record: serde_json::Value = serde_json::from_str(GNU).unwrap();
    let spec: serde_json::Value = serde_json::from_str(CASES).unwrap();
    assert_eq!(record["protocol"], "hash-read-syntax-v1");
    assert_eq!(record["gnu"], "GNU Emacs 32.0.50");
    assert_eq!(record["cases_sha256"], hex(&Sha256::digest(CASES)), "cases.json changed: re-record");
    assert_eq!(record["oracle_sha256"], hex(&Sha256::digest(ORACLE)), "oracle.py changed: re-record");
    let prelude = spec["prelude"].as_str().unwrap();
    let wrapper = spec["wrapper"].as_str().unwrap();
    let cases = record["cases"].as_array().unwrap();
    assert_eq!(cases.len(), spec["cases"].as_array().unwrap().len());
    assert!(cases.len() >= 80, "the corpus must not shrink");
    let pinned: BTreeMap<_, _> = DIVERGENT.iter().map(|(id, row, vm2)| (*id, (*row, *vm2))).collect();
    let mut failures = vec![];
    let mut seen = 0;
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let gnu = case["gnu"].as_str().unwrap();
        let source = wrapper.replace("%s", case["form"].as_str().unwrap());
        let actual = vm2_answer(prelude, &source);
        match pinned.get(id) {
            Some((row, expected)) => {
                seen += 1;
                if actual == gnu {
                    failures.push(format!("STALE PIN {id} ({row}): now matches GNU {gnu}"));
                } else if actual != *expected {
                    failures.push(format!("CHANGED {id} ({row}): pinned {expected}\n    now {actual}\n    gnu {gnu}"));
                }
            }
            None if actual != gnu => {
                failures.push(format!("NEW DIVERGENCE {id}:\n    gnu {gnu}\n    vm2 {actual}"))
            }
            None => {}
        }
    }
    assert_eq!(seen, pinned.len(), "a pinned case id is not in the corpus");
    assert!(failures.is_empty(), "{} of {} cases:\n{}", failures.len(), cases.len(), failures.join("\n"));
}

/// The reader builds a table from values it holds in `external_roots`, and an
/// `eq` table hashes cons and vector keys by heap index, which compaction
/// MOVES. Collection at every safepoint between the read and each lookup must
/// leave every key findable and the printed table intact (`gc-eq-cons-keys`
/// in the corpus is the same form against GNU). Garbage allocated before the
/// read and 200000 conses after it make a collection compact the keys
/// downward inside the window; the test asserts a collection happened and
/// the `moving`-key mutant (keys hashed as if they never move) kills it.
#[test]
fn a_read_eq_table_keeps_moving_keys_across_collections() {
    let spec: serde_json::Value = serde_json::from_str(CASES).unwrap();
    let record: serde_json::Value = serde_json::from_str(GNU).unwrap();
    let case = record["cases"].as_array().unwrap().iter()
        .find(|c| c["id"] == "gc-eq-cons-keys").unwrap();
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.eval_source(spec["prelude"].as_str().unwrap()).unwrap();
    let before = vm.heap_stats().collections;
    let source = format!("(prin1-to-string {})", case["form"].as_str().unwrap());
    // `display` takes `&Vm` and cannot collect; the `Local` must end before
    // `heap_stats` borrows the VM again.
    let shown = vm.eval_source(&source).unwrap().display().unwrap();
    let after = vm.heap_stats().collections;
    let shown: String = serde_json::from_str(&shown).unwrap();
    assert!(after > before, "the window must contain a collection ({before} -> {after})");
    assert_eq!(shown, case["gnu"].as_str().unwrap(), "keys lost or table corrupted across collections");
}

/// Reading and printing tables retains nothing once the forms return: no
/// reader roots, no unrooted signal data, no printer state. Measured through
/// `load_file` after a forced collection, in live objects AND bytes; the
/// third and later identical cycles must leave the heap exactly where the
/// second left it.
#[test]
fn reading_and_printing_tables_retains_no_objects_or_bytes() {
    let dir = std::env::temp_dir().join(format!("vm2-hash-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("hrs-cycle.el");
    std::fs::write(
        &file,
        concat!(
            ";;; -*- lexical-binding: t -*-\n",
            "(setq hrs-a (hash-table-count (car (read-from-string \"#s(hash-table test equal data (\\\"k\\\" (1 2) \\\"j\\\" [3 \\\"w\\\"] 2.5 x))\"))))\n",
            "(setq hrs-b (length (prin1-to-string (car (read-from-string \"#s(hash-table test eq data ((1) a [2] b))\")))))\n",
            "(setq hrs-c (condition-case e (read-from-string \"#s(hash-table data (a 1 b))\") (error (car e))))\n",
            "(setq hrs-d (condition-case e (read-from-string \"#s(hash-table test foo data (k v))\") (error (car e))))\n",
            "(setq hrs-e (condition-case e (read-from-string \"#s(foo 1 2)\") (error (car e))))\n",
        ),
    )
    .unwrap();
    let mut vm = Vm::new();
    let mut settle = |vm: &mut Vm| {
        vm.load_file(&file).unwrap();
        vm.collect_garbage();
        let stats = vm.heap_stats();
        (stats.live_objects, stats.live_bytes)
    };
    settle(&mut vm);
    let base = settle(&mut vm);
    eprintln!("hrs-cycle.el: live_objects={} live_bytes={} after a forced collection", base.0, base.1);
    for round in 0..4 {
        assert_eq!(settle(&mut vm), base, "round {round}: reading or printing a table retained heap state");
    }
    let value = vm.eval_source("(list hrs-a hrs-b hrs-c hrs-d hrs-e)").unwrap();
    assert_eq!(value.display().unwrap(), "(3 41 error error (#s(foo 1 2) . 11))");
}
