//! Complete live-GNU obarray contract, including domain, GC, alias and printer rows.
//! Opaque-obarray results retain their exact GNU assertion: printer mismatches
//! are reported explicitly as pending divergences, never silently skipped.
use romacs_vm::Vm;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn replay(evidence: &str, spec: &[u8]) {
    let evidence: Value = serde_json::from_str(evidence).unwrap();
    assert_eq!(evidence["protocol"], "primh-v1");
    assert_eq!(evidence["spec_sha256"], hash(spec));
    let spec_value: Value = serde_json::from_slice(spec).unwrap();
    let manifest: Value = serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/primitives.json"))).unwrap();
    let declaration = manifest.as_array().unwrap().iter().find(|row| row["name"] == spec_value["name"]).expect("primitive must be registered");
    assert_eq!(declaration["min"], spec_value["arity"][0]);
    assert_eq!(declaration["max"], spec_value["arity"][1]);
    // The file, not its line: a line moves whenever code above the declaration does, and two
    // branches that each move it would fail on their merge though neither is wrong.
    let file = |v: &Value| v.as_str().map(|s| s.rsplit_once(':').map_or(s, |(f, _)| f).to_owned());
    assert_eq!(file(&declaration["rust"]), file(&spec_value["rust"]));
    assert_eq!(declaration["gnu"], spec_value["gnu_def"]);
    assert_eq!(declaration["effect"].as_str().unwrap().to_lowercase(), spec_value["effect"].as_str().unwrap());
    assert_eq!(
        evidence["gnu"]["revision"],
        "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
    );
    assert_eq!(
        evidence["gnu_binary_sha256"],
        "a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8"
    );
    assert_eq!(
        evidence["generator_sha256"],
        hash(include_bytes!("../tools/primh/primh.py"))
    );
    assert_eq!(
        evidence["driver_sha256"],
        hash(include_bytes!("../tools/primh/gnu_driver.el"))
    );
    assert_eq!(
        evidence["domains_sha256"],
        hash(include_bytes!("../tools/primh/domains.json"))
    );
    assert_eq!(
        evidence["replay_sha256"],
        hash(include_bytes!("../tools/primh/replay.rs.in"))
    );
    let rows = evidence["cases"].as_array().unwrap();
    assert!(!rows.is_empty(), "empty oracle is not coverage");
    let mut checked = 0;
    let mut families = std::collections::BTreeMap::<String, usize>::new();
    let mut divergent_families = std::collections::BTreeMap::<String, usize>::new();
    let mut ids = std::collections::BTreeSet::new();
    let mut failures = Vec::new();
    for row in rows {
        assert!(ids.insert(row["id"].as_str().unwrap()), "duplicate case id");
        let family = row["family"].as_str().unwrap();
        *families.entry(family.to_owned()).or_default() += 1;
        checked += 1;
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let source = row["source"].as_str().unwrap();
        let actual = match vm.eval_source(source) {
            Ok(value) => match value.display() {
                Ok(text) => serde_json::from_str::<String>(&text).unwrap_or(text),
                Err(error) => format!("VM-ERROR {error}"),
            },
            Err(error) => format!("VM-ERROR {error}"),
        };
        if row["expected"].as_str() != Some(&actual)
            && row["gnu_regime_variant"].as_str() != Some(&actual)
        {
            let kind = if row["expected"].as_str().unwrap().contains("#<obarray") {
                "opaque-obarray printer: pending exact parity"
            } else {
                family
            };
            *divergent_families.entry(kind.to_owned()).or_default() += 1;
            failures.push(format!(
                "{} [{kind}]: {source}\nGNU {}\nVM2 {actual}",
                row["id"], row["expected"]
            ));
        }
    }
    assert_eq!(checked, rows.len(), "every recorded case must be replayed");
    eprintln!(
        "{}: replayed {checked} rows; families={families:?}; pending={divergent_families:?}",
        evidence["primitive"]
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

macro_rules! contract {
    ($test:ident, $name:literal) => {
        #[test]
        fn $test() {
            replay(
                include_str!(concat!("../evidence/obarray-substrate/", $name, ".json")),
                include_bytes!(concat!(
                    "../evidence/obarray-substrate/specs/",
                    $name,
                    ".json"
                )),
            );
        }
    };
}

contract!(obarray_make, "obarray-make");
contract!(obarray_predicate_distinguishes_vectors, "obarrayp");
contract!(intern_custom_identity_and_legacy_vector_mutation, "intern");
contract!(intern_soft_identity_and_misses, "intern-soft");
contract!(unintern_removal_requires_matching_identity, "unintern");
contract!(mapatoms_exact_gnu_order_and_coverage, "mapatoms");
