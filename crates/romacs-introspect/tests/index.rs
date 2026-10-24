//! The embedded index answers the questions this project kept getting wrong,
//! and refuses, with reasons, the ones it cannot support.

use romacs_introspect::{
    Arity, Cause, CoreSubr, Freshness, Implementation, Index, Lookup, MaxArity, Registry, Reverse,
    SetQuery, SnapshotVerdict,
};

fn found(name: &str) -> Box<romacs_introspect::Described> {
    match Index::embedded().describe(name) {
        Lookup::Found(d) => d,
        Lookup::NotFound(a) => panic!("{name}: expected Found, got NotFound: {a:?}"),
    }
}

fn not_found(index: &Index, name: &str) -> Box<romacs_introspect::Absence> {
    match index.describe(name) {
        Lookup::NotFound(a) => a,
        Lookup::Found(d) => panic!(
            "{name}: expected NotFound, got Found: {:?}",
            d.implementation
        ),
    }
}

// ------------------------------------------------------------ point lookup

/// With core retired, `car` belongs exclusively to VM2.
#[test]
fn car_is_implemented_by_vm2_only() {
    let d = Index::embedded().describe("car");
    let Lookup::Found(d) = d else {
        panic!("car not found")
    };
    assert_eq!(d.implementation, Implementation::Vm2Only);
    assert!(d.core.is_empty());
    assert!(d.arity.is_none());
}

fn source_line(span: &romacs_introspect::Span, prefix: &str) -> String {
    assert!(span.file.starts_with(prefix), "span {span}");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = std::fs::read_to_string(root.join(&span.file)).expect("span file exists");
    let line = source
        .lines()
        .nth(span.line.checked_sub(1).expect("positive span line"))
        .expect("span line exists");
    assert!(!line.trim().is_empty(), "nonempty source at {span}");
    line.to_owned()
}

fn vm2_regexp_span(d: &romacs_introspect::Described, variant: &str) {
    assert_eq!(d.vm2.len(), 1, "one VM2 registration for {}", d.name);
    let v = &d.vm2[0];
    assert_eq!(v.kind, "primitive-table");
    assert_eq!(
        v.variant,
        format!("Primitive::{variant}"),
        "regexp VM2 variant for {}",
        d.name
    );
    assert!(source_line(&v.site, "crates/romacs-vm/src/").contains(&d.name));
    let handler = v.handler.as_ref().expect("VM2 handler span");
    assert!(
        source_line(handler, "crates/romacs-vm/src/").contains(variant),
        "handler {handler}"
    );
}

/// Core retirement must not leave a phantom registration or invented VM2 arity.
#[test]
fn re_search_forward_is_vm2_only_with_resolving_spans() {
    let d = found("re-search-forward");
    assert_eq!(d.implementation, Implementation::Vm2Only);
    assert!(d.core.is_empty());
    assert!(d.arity.is_none(), "VM2 exposes no arity table");
    vm2_regexp_span(&d, "ReSearchForward");
}

#[test]
fn regexp_primitives_have_exact_runtime_ownership_and_resolving_spans() {
    for (name, variant) in [
        ("looking-at", "LookingAt"),
        ("string-match", "StringMatch"),
        ("re-search-forward", "ReSearchForward"),
        ("re-search-backward", "ReSearchBackward"),
    ] {
        let d = found(name);
        assert_eq!(d.implementation, Implementation::Vm2Only, "{name}");
        assert!(d.core.is_empty(), "retired core registration for {name}");
        assert!(d.arity.is_none(), "invented VM2 arity for {name}");
        vm2_regexp_span(&d, variant);
    }
    // match-string was a Lisp definition, not a core subr. Do not invent core ownership.
    let d = found("match-string");
    assert_eq!(d.implementation, Implementation::Vm2Only);
    assert!(d.core.is_empty());
    assert!(d.lisp_definition.is_some());
    vm2_regexp_span(&d, "MatchString");
}

/// `not` is bound outside VM2's PRIMITIVES table, by a direct slot write.
#[test]
fn not_is_found_in_vm2_through_its_direct_slot() {
    let d = Index::embedded().describe("not");
    let Lookup::Found(d) = d else {
        panic!("not not found")
    };
    assert!(
        d.vm2.iter().any(|v| v.kind == "direct-slot"),
        "vm2: {:?}",
        d.vm2
    );
}

/// R98: help that reads the ledger. GNU's behaviour beside ours, and whether
/// a package can observe it.
#[test]
fn maphash_carries_its_observable_divergence_with_gnu_beside_ours() {
    let d = Index::embedded().describe("maphash");
    let Lookup::Found(d) = d else {
        panic!("maphash not found")
    };
    let row = d
        .divergences
        .iter()
        .find(|r| r.observable)
        .expect("an observable divergence row for maphash");
    assert!(
        !row.gnu.is_empty() && !row.romacs.is_empty(),
        "both behaviours: {row:?}"
    );
    assert!(row.ledger.line > 0);
}

#[test]
fn special_form_if_is_not_falsely_reported_as_a_registered_core_subr() {
    // VM2 compiles `if` as syntax, not a funcall-able registry entry.
    let a = not_found(Index::embedded(), "if");
    assert_eq!(a.coverage.core_registered, 0);
    assert!(matches!(
        Index::embedded().implements("eval_if"),
        Reverse::NotFound { .. }
    ));
}

// --------------------------------------------------------- honest negatives

/// `string-trim` is Lisp. The answer must say so, not "does not exist".
#[test]
fn a_lisp_function_is_not_found_and_says_where_the_lisp_is() {
    let a = not_found(Index::embedded(), "string-trim");
    let lisp = a.likely_causes.iter().find_map(|c| match c {
        Cause::LispDefined { at, .. } => Some(at.clone()),
        _ => None,
    });
    let at = lisp.expect("a LispDefined cause");
    assert!(
        at.file.starts_with("gnu-lisp/") && at.file.ends_with(".el"),
        "{at}"
    );
}

#[test]
fn a_typo_is_not_found_and_offers_the_near_miss() {
    let a = not_found(Index::embedded(), "mapash");
    let near = a.likely_causes.iter().find_map(|c| match c {
        Cause::NearMiss { names } => Some(names.clone()),
        _ => None,
    });
    assert!(
        near.expect("a NearMiss cause")
            .contains(&"maphash".to_string())
    );
}

#[test]
fn an_unknown_name_states_its_coverage_and_never_claims_absence() {
    let a = not_found(Index::embedded(), "zq-no-such-function-anywhere");
    assert!(
        a.coverage.core_registered == 0 && a.coverage.vm2_registered > 100,
        "coverage stated: {:?}",
        a.coverage
    );
    assert!(a.likely_causes.contains(&Cause::NoTrace));
    let text = Index::embedded()
        .describe("zq-no-such-function-anywhere")
        .to_string();
    assert!(text.contains("NOT FOUND"), "{text}");
    assert!(text.contains("coverage"), "{text}");
    assert!(
        !text.contains("does not exist"),
        "a refusal must not assert absence: {text}"
    );
}

// --------------------------------------------------------------- set queries

#[test]
fn implementation_sets_partition_the_registry() {
    let ix = Index::embedded();
    let vm2 = ix
        .query(&[SetQuery::Implementation(Implementation::Vm2Only)])
        .names;
    let core = ix
        .query(&[SetQuery::Implementation(Implementation::CoreOnly)])
        .names;
    let both = ix
        .query(&[SetQuery::Implementation(Implementation::Both)])
        .names;
    assert_eq!(vm2.len() + core.len() + both.len(), ix.names().len());
    assert!(ix.names().len() > 100);
    assert!(core.is_empty(), "retired core-only names: {core:?}");
    assert!(both.is_empty(), "retired shared ownership: {both:?}");
    assert_eq!(
        vm2.iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        ix.names()
    );
    assert!(vm2.contains(&"maphash".to_string()));
    assert!(vm2.contains(&"re-search-forward".to_string()));
    assert!(vm2.contains(&"eval-cache-stats".to_string()));
}

#[test]
fn observable_divergent_primitives_include_maphash_and_report_what_was_excluded() {
    let a = Index::embedded().query(&[SetQuery::Divergent {
        observable_only: true,
    }]);
    assert!(a.names.contains(&"maphash".to_string()));
    assert!(!a.names.contains(&"car".to_string()));
    let all = Index::embedded().query(&[SetQuery::Divergent {
        observable_only: false,
    }]);
    let expected: Vec<_> = all
        .names
        .iter()
        .filter(|n| found(n).divergences.iter().any(|row| row.observable))
        .cloned()
        .collect();
    assert_eq!(
        a.names, expected,
        "observable filter over the VM2 population"
    );
    for n in &a.names {
        let Lookup::Found(d) = Index::embedded().describe(n) else {
            panic!("{n}")
        };
        assert!(
            d.divergences.iter().any(|r| r.observable),
            "{n} has no observable row"
        );
    }
    assert!(
        a.caveats
            .iter()
            .any(|c| c.contains("not registered primitives")),
        "ledger symbols that are not primitives must be named, not dropped: {:?}",
        a.caveats
    );
}

#[test]
fn untested_set_is_lexical_and_says_so() {
    let ix = Index::embedded();
    let a = ix.query(&[SetQuery::NoTestMentions]);
    assert!(!a.names.contains(&"car".to_string()));
    assert!(!a.names.is_empty());
    for n in a.names.iter().take(20) {
        let Lookup::Found(d) = ix.describe(n) else {
            panic!("{n}")
        };
        assert_eq!(d.tests.files, 0, "{n}");
    }
    assert!(
        a.caveats.iter().any(|c| c.contains("lexical")),
        "{:?}",
        a.caveats
    );
}

// ---------------------------------------------------------------- reverse

#[test]
fn rust_fn_variant_and_span_map_back_to_lisp_names() {
    let ix = Index::embedded();
    let names = |r: Reverse| match r {
        Reverse::Found { names, .. } => names.into_iter().map(|i| i.lisp_name).collect::<Vec<_>>(),
        Reverse::NotFound { query, .. } => panic!("{query}: not found"),
    };
    assert!(matches!(
        ix.implements("builtin_maphash"),
        Reverse::NotFound { .. }
    ));
    assert!(names(ix.implements("Primitive::Maphash")).contains(&"maphash".to_string()));
    let d = found("maphash");
    let handler = d.vm2[0].handler.as_ref().expect("maphash VM2 handler");
    assert!(source_line(handler, "crates/romacs-vm/src/").contains("Maphash"));
    let span = handler.to_string();
    assert!(names(ix.implements(&span)).contains(&"maphash".to_string()));
}

#[test]
fn an_unknown_rust_fn_is_not_found_with_what_was_searched() {
    match Index::embedded().implements("builtin_no_such_thing_q") {
        Reverse::NotFound {
            searched, coverage, ..
        } => {
            assert!(!searched.is_empty());
            assert_eq!(coverage.core_registered, 0);
            assert!(coverage.vm2_registered > 100);
        }
        Reverse::Found { names, .. } => panic!("found {names:?}"),
    }
}

// --------------------------------------------------------------- coverage

/// Neither the snapshot nor derived coverage may retain retired core entries.
#[test]
fn retired_core_has_no_registry_entries_or_registration_sites() {
    let c = Index::embedded().coverage();
    assert!(Index::embedded().registry().core.is_empty());
    assert_eq!(c.core_registered, 0);
    assert_eq!(c.core_with_fn_span, 0);
    assert!(
        !Index::embedded()
            .registry()
            .provenance
            .source_only_at_generation
            .iter()
            .any(|n| n.starts_with("core:"))
    );
    assert!(
        c.core_without_site.is_empty(),
        "registered with no recognised site: {:?}",
        c.core_without_site
    );
    assert_eq!(c.core_with_site, c.core_registered);
}

#[test]
fn every_registered_vm2_function_has_a_site() {
    let c = Index::embedded().coverage();
    assert!(c.vm2_registered > 100);
    assert!(
        c.vm2_without_site.is_empty(),
        "VM2 registered with no site: {:?}",
        c.vm2_without_site
    );
}

// -------------------------------------------------------------- staleness

/// A snapshot older than the code: the source registers `car` but the
/// snapshot does not. The index must call itself stale and refuse `car`.
#[test]
fn a_snapshot_older_than_the_source_is_stale_and_refuses_the_drifted_name() {
    let mut old: Registry = Index::embedded().registry().clone();
    old.core.retain(|c| c.name != "car");
    old.vm2.retain(|n| n != "car");
    let ix = Index::with_snapshot(old);
    match &ix.provenance().freshness {
        Freshness::Snapshot(SnapshotVerdict::Stale { source_registers }) => {
            assert!(
                source_registers.contains(&"car".to_string()),
                "{source_registers:?}"
            )
        }
        other => panic!("expected Stale, got {other:?}"),
    }
    let a = not_found(&ix, "car");
    assert!(
        a.likely_causes
            .iter()
            .any(|c| matches!(c, Cause::RegisteredInSourceOnly { .. })),
        "{:?}",
        a.likely_causes
    );
    // Other answers still come back, but carry the staleness.
    let Lookup::Found(m) = ix.describe("maphash") else {
        panic!("maphash")
    };
    assert!(m.provenance.freshness.is_stale());
}

/// Drift remains per-runtime even for an externally supplied historical
/// registry: a stale core entry must not mask a missing VM2 registration.
#[test]
fn a_snapshot_missing_a_name_from_one_runtime_only_is_stale() {
    let mut old: Registry = Index::embedded().registry().clone();
    // Synthetic historical entry preserves the union while VM2 alone drifts.
    old.core.push(CoreSubr {
        name: "car".into(),
        subr_name: "car".into(),
        arity: Arity {
            min: 1,
            max: MaxArity::Fixed(1),
        },
    });
    old.vm2.retain(|n| n != "car");
    let ix = Index::with_snapshot(old);
    match &ix.provenance().freshness {
        Freshness::Snapshot(SnapshotVerdict::Stale { source_registers }) => {
            assert!(
                source_registers.contains(&"car".to_string()),
                "{source_registers:?}"
            )
        }
        other => panic!("expected Stale, got {other:?}"),
    }
    // The old core entry cannot substitute for the source's VM2 ownership.
    let a = not_found(&ix, "car");
    assert!(
        a.likely_causes
            .iter()
            .any(|c| matches!(c, Cause::RegisteredInSourceOnly { .. })),
        "{:?}",
        a.likely_causes
    );
}

#[test]
fn the_committed_snapshot_is_not_stale_against_this_build() {
    assert!(
        Index::embedded().drift().is_empty(),
        "drift: {:?}",
        Index::embedded().drift()
    );
    assert!(!Index::embedded().provenance().freshness.is_stale());
}

#[test]
fn a_live_registry_is_live() {
    let mut reg = Index::embedded().registry().clone();
    reg.provenance.generated_at_unix = 1_800_000_000;
    let ix = Index::with_live_registry(reg);
    assert_eq!(
        ix.provenance().freshness,
        Freshness::Live {
            enumerated_at_unix: 1_800_000_000
        }
    );
}

// ------------------------------------------------------------- provenance

#[test]
fn every_answer_carries_provenance() {
    let ix = Index::embedded();
    let p = &found("car").provenance;
    assert!(p.derived_at_unix > 1_700_000_000, "derivation time: {p:?}");
    assert!(!p.git.is_empty());
    assert_eq!(p.registry.input_hash.as_deref().map(str::len), Some(64));
    assert!(not_found(ix, "zq-nope").provenance.derived_at_unix > 1_700_000_000);
    assert!(ix.query(&[SetQuery::All]).provenance.derived_at_unix > 1_700_000_000);
}

#[test]
fn the_json_answer_is_structured_and_self_describing() {
    let j = Index::embedded().describe("maphash").to_json();
    assert_eq!(j["found"], true);
    assert_eq!(j["implementation"], "vm2");
    assert_eq!(j["core"], serde_json::json!([]));
    assert!(j["arity"].is_null());
    assert!(
        j["provenance"]["derived_at_unix"].as_u64().unwrap_or(0) > 1_700_000_000,
        "{j}"
    );
    assert!(j["divergences"].as_array().is_some_and(|a| !a.is_empty()));
    let n = Index::embedded().describe("zq-nope").to_json();
    assert_eq!(n["found"], false);
    assert!(
        n["coverage"].is_object() && n["searched"].is_string(),
        "{n}"
    );
}

#[test]
fn current_requires_matching_v2_input_hash_and_platform() {
    let mut original = Index::embedded().registry().clone();
    original.provenance.input_hash = Index::embedded().provenance().build_input_hash.clone();
    original.provenance.generated_on =
        format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    assert_eq!(
        Index::with_snapshot(original.clone())
            .provenance()
            .freshness,
        Freshness::Snapshot(SnapshotVerdict::Current)
    );
    for variant in ["hash", "platform", "legacy", "invalid"] {
        let mut json = original.to_json();
        match variant {
            "hash" => json["provenance"]["input_hash"] = serde_json::json!("0".repeat(64)),
            "platform" => json["provenance"]["generated_on"] = serde_json::json!("other-platform"),
            "legacy" => json["schema"] = serde_json::json!(1),
            _ => json["provenance"]["input_hash"] = serde_json::json!("not-a-sha256"),
        }
        let registry = Registry::from_json(&json).expect("legacy remains readable");
        assert_eq!(
            Index::with_snapshot(registry).provenance().freshness,
            Freshness::Snapshot(SnapshotVerdict::NoDriftDetected),
            "{variant}"
        );
    }
}
