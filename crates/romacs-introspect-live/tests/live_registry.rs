//! Function-cell enumeration, including registrations outside the primitive table.
use romacs_introspect_live::{enumerate, vm2_registry};
use std::collections::BTreeSet;

/// `not` is bound by a direct slot assignment, outside VM2's PRIMITIVES table.
#[test]
fn vm2_probe_finds_names_bound_outside_the_table() {
    let universe: BTreeSet<String> = [
        "car",
        "not",
        "maphash",
        "looking-at",
        "string-match",
        "match-string",
        "re-search-forward",
        "re-search-backward",
        "surely-not-a-function-q7",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let (found, _panics) = vm2_registry(&universe);
    let found: BTreeSet<&str> = found.iter().map(String::as_str).collect();
    assert_eq!(
        found,
        [
            "car",
            "maphash",
            "not",
            "looking-at",
            "string-match",
            "match-string",
            "re-search-forward",
            "re-search-backward"
        ]
        .into_iter()
        .collect::<BTreeSet<_>>(),
        "VM2 live regexp registrations"
    );
}

#[test]
fn enumerate_records_provenance() {
    let r = enumerate().expect("enumerate");
    let hash = r
        .provenance
        .input_hash
        .as_deref()
        .expect("input fingerprint recorded");
    assert_eq!(hash.len(), 64);
    assert!(
        hash.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(
        hash,
        romacs_introspect::fingerprint::input_hash(&root).unwrap()
    );
    assert_eq!(
        Some(hash),
        romacs_introspect::Index::embedded()
            .provenance()
            .build_input_hash
            .as_deref()
    );
    assert!(
        r.provenance.vm2_universe >= r.vm2.len() && !r.vm2.is_empty(),
        "VM2 universe recorded"
    );
    assert!(
        r.core.is_empty() && r.provenance.core_method.contains("Retired"),
        "method recorded: {}",
        r.provenance.core_method
    );
}

#[test]
fn enumeration_does_not_invoke_candidates() {
    let universe = [
        "kill-emacs",
        "error",
        "car",
        "and",
        "surely-not-a-function-q7",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let (found, failures) = vm2_registry(&universe);
    assert!(failures.is_empty(), "{failures:?}");
    assert!(found.contains(&"car".to_owned()));
    assert!(found.contains(&"error".to_owned()));
    // GNU's special forms are subrs in their function cells: fboundp is t.
    assert!(found.contains(&"and".to_owned()));
    assert!(!found.contains(&"surely-not-a-function-q7".to_owned()));
}
