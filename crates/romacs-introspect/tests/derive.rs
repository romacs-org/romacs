//! The derivation rules, on fixtures. `build/derive.rs` is compiled here
//! exactly as `build.rs` compiles it.

#[path = "../build/derive.rs"]
mod derive;

use derive::*;
use std::collections::BTreeMap;
use std::path::Path;

fn file(rel: &str, text: &str) -> SourceFile {
    SourceFile::new(rel.to_string(), text.to_string())
}

fn site<'a>(sites: &'a [CoreSite], name: &str) -> &'a CoreSite {
    sites
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no site for {name}: {sites:?}"))
}

const SHAPES: &str = r#"
fn register(ctx: &mut Context) {
    ctx.register_subr(SubrSpec::new(
        "maphash",
        NativeFn::ContextVec(crate::emacs_core::hashtab::builtin_maphash),
        SubrArity::new(2, Some(2)),
    ));
    ctx.register_subr(SubrSpec::fixed4(
        "re-search-forward",
        builtin_re_search_forward_4,
        FixedMin4::One,
    ));
    register_pure_subr!(ctx, "take", crate::emacs_core::builtins_extra::builtin_take, 2, Some(2));
    ctx.register_subr(SubrSpec::new("plus", NativeFn::Pure(|args| builtin_plus(args)), SubrArity::new(0, None)));
}
const EVALUATOR_SUBRS: &[EvaluatorSubr] = &[
    EvaluatorSubr::callable("throw", SubrArity::new(2, Some(2)), CallableHandler::Throw),
    EvaluatorSubr::special("if", SubrArity::new(2, None), SpecialFormHandler::If),
];
/// const BAD: SubrSpec = SubrSpec::fixed2("bad", one, FixedMin2::One);
const NEXT: SubrSpec = SubrSpec::lazy("future-shape", builtin_future);
"#;

#[test]
fn every_known_core_shape_yields_its_site_and_implementation() {
    let sites = core_sites(&[file("crates/romacs-core/src/x.rs", SHAPES)]);
    let m = site(&sites, "maphash");
    assert_eq!(
        (m.shape.as_str(), m.impl_ref.as_deref()),
        (
            "SubrSpec::new",
            Some("crate::emacs_core::hashtab::builtin_maphash")
        )
    );
    assert_eq!(m.line, 3);
    let r = site(&sites, "re-search-forward");
    assert_eq!(
        (r.shape.as_str(), r.impl_ref.as_deref()),
        ("SubrSpec::fixed4", Some("builtin_re_search_forward_4"))
    );
    assert_eq!(site(&sites, "take").shape, "register_pure_subr!");
    let p = site(&sites, "plus");
    assert_eq!(
        (p.impl_kind, p.impl_ref.as_deref()),
        ("closure", Some("builtin_plus"))
    );
    // `SubrArity::new(2, Some(2))` has nested parentheses; `throw` was once
    // missed for exactly that.
    assert_eq!(
        site(&sites, "throw").impl_ref.as_deref(),
        Some("CallableHandler::Throw")
    );
    assert_eq!(site(&sites, "if").shape, "EvaluatorSubr::special");
}

/// A constructor nobody taught the derivation still yields a site: the
/// generic `SubrSpec::<ctor>("name"` fallback. Its implementation is labelled
/// unparsed rather than guessed.
#[test]
fn an_unknown_subrspec_constructor_still_yields_a_site() {
    let sites = core_sites(&[file("crates/romacs-core/src/x.rs", SHAPES)]);
    let f = site(&sites, "future-shape");
    assert_eq!(f.shape, "SubrSpec::lazy");
    assert_eq!(f.impl_kind, "unparsed");
}

#[test]
fn a_doc_comment_example_is_not_a_registration() {
    let sites = core_sites(&[file("crates/romacs-core/src/x.rs", SHAPES)]);
    assert!(!sites.iter().any(|s| s.name == "bad"), "{sites:?}");
}

#[test]
fn test_sources_are_never_registration_sites() {
    for p in [
        "crates/romacs-core/src/a/tests/b.rs",
        "crates/romacs-core/src/a/b_test.rs",
        "crates/romacs-core/src/a/tests.rs",
    ] {
        assert!(is_test_path(p), "{p}");
    }
    assert!(!is_test_path(
        "crates/romacs-core/src/emacs_core/hashtab/mod.rs"
    ));
}

#[test]
fn several_fns_of_one_name_are_reported_ambiguous_not_guessed() {
    let mut defs: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    defs.insert(
        "builtin_x".into(),
        vec![
            ("crates/c/src/a/one.rs".into(), 10),
            ("crates/c/src/b/two.rs".into(), 20),
        ],
    );
    let r = resolve_fn("builtin_x", "crates/c/src/z/site.rs", &defs);
    assert_eq!(r["resolution"], "ambiguous", "{r}");
    assert!(r["span"].is_null());
    assert_eq!(r["candidates"].as_array().unwrap().len(), 2);
    // a module path that names one of them settles it, and says how
    let r = resolve_fn("crate::b::builtin_x", "crates/c/src/z/site.rs", &defs);
    assert_eq!(
        (r["resolution"].as_str(), r["span"].as_str()),
        (Some("module-path"), Some("crates/c/src/b/two.rs:20"))
    );
    // a bare name defined in the registering file resolves there
    defs.get_mut("builtin_x")
        .unwrap()
        .push(("crates/c/src/z/site.rs".into(), 5));
    let r = resolve_fn("builtin_x", "crates/c/src/z/site.rs", &defs);
    assert_eq!(r["resolution"], "same-file");
}

const VM2: &str = r#"
const PRIMITIVES: &[(&str, Primitive)] = &[
    ("car", Primitive::Car),
    (
        "hash-table-rehash-threshold",
        Primitive::HashTableRehashThreshold,
    ),
];
fn new() {
        let not = vm.slot("not");
        vm.slots[not].function = Value::reference(Primitive::Null as usize, 6);
}
fn annotation_definition(name: &str) -> Option<serde_json::Value> {
    Some(match name {
        "kill-region" => serde_json::json!([
            ["beg", "end"],
        ]),
        _ => return None,
    })
}
"#;

#[test]
fn vm2_table_entries_direct_slots_and_lisp_bodies_are_all_sites() {
    let sites = vm2_sites(&[file("crates/romacs-vm/src/lib.rs", VM2)]);
    let kinds: BTreeMap<&str, &str> = sites.iter().map(|s| (s.name.as_str(), s.kind)).collect();
    assert_eq!(kinds.get("car"), Some(&"primitive-table"));
    // rustfmt splits long entries over lines; that once hid three names
    assert_eq!(
        kinds.get("hash-table-rehash-threshold"),
        Some(&"primitive-table")
    );
    assert_eq!(kinds.get("not"), Some(&"direct-slot"));
    assert_eq!(kinds.get("kill-region"), Some(&"lisp-definition"));
}

#[test]
fn test_mentions_are_calls_and_function_quotes() {
    let mut seen = Default::default();
    mentions(
        r#"(should (equal (maphash #'ignore h) nil)) "(car x)""#,
        &mut seen,
    );
    for n in ["should", "equal", "maphash", "ignore", "car"] {
        assert!(seen.contains(n), "{n}: {seen:?}");
    }
}

const LEDGER: &str = "\
## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| D001 | string mutation | bytes | scalars | correctness |  <!-- symbols: aset, store-substring -->
| H1 | indirect `maphash` | calls | signals `error` | VM2 cannot call Lisp |
| — | moved | a | b | c |  <!-- not-a-divergence -->

## Not observable

| — | `case_table::translate` cache | x | y | z |
";

#[test]
fn ledger_rows_carry_symbols_observability_and_line() {
    let rows = parse_ledger(LEDGER, "DIVERGENCES.md");
    assert_eq!(rows.len(), 3, "{rows:#?}");
    assert_eq!(
        rows[0]["symbols"],
        serde_json::json!(["aset", "store-substring"])
    );
    assert_eq!(rows[0]["symbols_derived"], false);
    assert_eq!(rows[1]["symbols"], serde_json::json!(["maphash", "error"]));
    assert_eq!(rows[1]["observable"], true);
    assert_eq!(rows[1]["line"], 6);
    assert_eq!(rows[2]["observable"], false);
    assert_eq!(
        rows[2]["symbols"],
        serde_json::json!([]),
        "a Rust path is not a Lisp symbol"
    );
}

/// The Rust port of `tools/divergence_index.py` must agree with what that
/// script last wrote. If this fails, either the port drifted from the script
/// or `DIVERGENCES.json` is stale against the ledgers -- both are worth
/// knowing, and the message says which side to regenerate.
#[test]
fn the_rust_ledger_port_agrees_with_divergences_json() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let Ok(text) = std::fs::read_to_string(root.join("DIVERGENCES.json")) else {
        panic!("DIVERGENCES.json missing at the workspace root");
    };
    let py: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut rows = Vec::new();
    for (rel, p) in ledgers(&root) {
        rows.extend(parse_ledger(&std::fs::read_to_string(p).unwrap(), &rel));
    }
    let key = |r: &serde_json::Value| {
        (
            r["source"].clone(),
            r["mechanism"].clone(),
            r["symbols"].clone(),
            r["observable"].clone(),
        )
    };
    let ours: Vec<_> = rows.iter().map(key).collect();
    let theirs: Vec<_> = py["rows"].as_array().unwrap().iter().map(key).collect();
    assert_eq!(
        ours, theirs,
        "the ledger port and DIVERGENCES.json disagree; if the .md files changed, run python3 tools/divergence_index.py"
    );
}
