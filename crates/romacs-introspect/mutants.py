#!/usr/bin/env python3
"""Re-run every mutant listed in MUTANTS.md and report which tests kill it.

    python3 crates/romacs-introspect/mutants.py            # all
    python3 crates/romacs-introspect/mutants.py M06 M08    # some

Each mutant replaces one exact string in one file, runs the named test target,
and counts as KILLED only if the expected test is reported FAILED. The file is
restored from memory in a `finally`, so an interrupted run leaves no mutant
behind; `git status` should be clean afterwards either way. Exit 1 if any
mutant survives. Run inside the lane environment (pinned toolchain, private
CARGO_TARGET_DIR); it builds incrementally against that target.
"""
from __future__ import annotations
import pathlib, subprocess, sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
I = 'crates/romacs-introspect'
L = 'crates/romacs-introspect-live'

# (id, file, old, new, cargo args, test expected to FAIL, what it protects)
MUTANTS = [
    ('M01', f'{I}/build/derive.rs', 'r"register_pure_subr!\\(', 'r"register_pure_subr_RENAMED!\\(',
     ['-p', 'romacs-introspect', '--test', 'index'], 'every_registered_core_subr_has_a_registration_site',
     'a registration shape the derivation stops recognising is named, not hidden'),
    ('M02', f'{I}/build/derive.rs', r'SubrArity::new\((?:[^()]|\([^()]*\))*\)', r'SubrArity::new\([^)]*\)',
     ['-p', 'romacs-introspect', '--test', 'index'], 'every_registered_core_subr_has_a_registration_site',
     '`throw` (nested parens in its arity) keeps its site'),
    ('M03', f'{I}/build/derive.rs', r'(Primitive::\w+)\s*,?\s*\)', r'(Primitive::\w+)\s*\)',
     ['-p', 'romacs-introspect', '--test', 'index'], 'every_registered_vm2_function_has_a_site',
     'rustfmt-split VM2 table entries keep their site'),
    ('M04', f'{I}/build/derive.rs', 'if f.in_comment(start) || !taken.insert(start) {', 'if !taken.insert(start) {',
     ['-p', 'romacs-introspect', '--test', 'derive'], 'a_doc_comment_example_is_not_a_registration',
     'doc-comment examples are not registrations'),
    ('M05', f'{I}/build/derive.rs', '    if cands.len() == 1 {', '    if !cands.is_empty() {',
     ['-p', 'romacs-introspect', '--test', 'derive'], 'several_fns_of_one_name_are_reported_ambiguous_not_guessed',
     'an ambiguous fn is reported, never silently the first'),
    ('M06', f'{I}/src/lib.rs',
     ['.filter(|n| !core.contains_key(n) && !expected.contains(format!("core:{n}").as_str()))',
      '!vm2.contains(n) && !expected.contains(format!("vm2:{n}").as_str())'],
     ['.filter(|_| false)', 'false'],
     ['-p', 'romacs-introspect', '--test', 'index'], 'a_snapshot_older_than_the_source_is_stale_and_refuses_the_drifted_name',
     'drift between source and snapshot is computed'),
    ('M34', f'{I}/src/lib.rs',
     '!vm2.contains(n) && !expected.contains(format!("vm2:{n}").as_str())',
     '!vm2.contains(n) && !core.contains_key(n) && !expected.contains(format!("vm2:{n}").as_str())',
     ['-p', 'romacs-introspect', '--test', 'index'], 'a_snapshot_missing_a_name_from_one_runtime_only_is_stale',
     'drift is per runtime: VM2 gaining a name core has (integerp, after the fixnum merge) is staleness'),
    ('M35', f'{I}/src/lib.rs', '        if self.drift.iter().any(|d| d == name) {', '        if self.drift.iter().any(|d| d == name) && false {',
     ['-p', 'romacs-introspect', '--test', 'index'], 'a_snapshot_missing_a_name_from_one_runtime_only_is_stale',
     'a partially drifted name is refused, not answered with the stale implementation'),
    ('M07', f'{I}/src/lib.rs', 'None if !drift.is_empty() => Freshness::Snapshot(', 'None if false => Freshness::Snapshot(',
     ['-p', 'romacs-introspect', '--test', 'index'], 'a_snapshot_older_than_the_source_is_stale_and_refuses_the_drifted_name',
     'a drifted snapshot calls itself stale'),
    ('M08', f'{I}/data/registry.json', '"name": "car"\n', '"name": "car-REMOVED"\n',
     ['-p', 'romacs-introspect-live', '--test', 'snapshot'], 'snapshot_matches_live_registry',
     'the gate catches a snapshot that lost a primitive'),
    ('M09', f'{I}/data/registry.json', '"name": "re-search-forward"\n', '"name": "re-search-forward-REMOVED"\n',
     ['-p', 'romacs-introspect', '--test', 'index'], 'the_committed_snapshot_is_not_stale_against_this_build',
     'the binary itself notices a snapshot older than its source, with no gate'),
    ('M10', f'{I}/data/registry.json', '"name": "re-search-forward"\n', '"name": "re-search-forward-X"\n',
     ['-p', 'romacs-introspect-live', '--test', 'snapshot'], 'snapshot_matches_live_registry',
     'a renamed/extra snapshot entry is caught'),
    ('M11', f'{L}/src/lib.rs',
     ['    let mut found = Vec::new();\n', '            let mut vm = Vm::new();\n            let f = vm.compile'],
     ['    let mut found = Vec::new();\n    let mut vm = Vm::new();\n', '            let f = vm.compile'],
     ['-p', 'romacs-introspect-live', '--test', 'snapshot'], 'snapshot_matches_live_registry',
     'EQUIVALENT: one VM per probe is defence; with funcall probes a shared VM gives the same registry today'),
    ('M12', f'{L}/src/lib.rs', 'Form::Sym("funcall".into()),\n            Form::List(vec![Form::Sym("quote".into()), Form::Sym(name.clone())]),',
     'Form::Sym(name.clone()),',
     ['-p', 'romacs-introspect-live', '--test', 'snapshot'], 'snapshot_matches_live_registry',
     'the VM2 probe must not count compiler-expanded forms (and, cond) as functions'),
    ('M13', f'{I}/src/registry.rs', 'n => n.parse().ok().map(MaxArity::Fixed),', 'n => n.parse::<u16>().ok().map(|n| MaxArity::Fixed(n + 1)),',
     ['-p', 'romacs-introspect-live', '--test', 'live_registry'], 'core_registry_has_fixed4_registration_with_its_arity',
     'core arity is what subr-arity reports'),
    ('M14', f'{I}/src/registry.rs', '"unevalled" => Some(MaxArity::Unevalled),', '"unevalled" => Some(MaxArity::Many),',
     ['-p', 'romacs-introspect-live', '--test', 'live_registry'], 'special_forms_are_unevalled',
     'special forms are reported as such'),
    ('M15', f'{I}/src/lib.rs', '        if let Some((at, form)) = self.lisp_definition(name) {\n            causes.push(Cause::LispDefined { at, form });',
     '        if let Some((_at, _form)) = self.lisp_definition(name) {\n            causes.push(Cause::NoTrace);',
     ['-p', 'romacs-introspect', '--test', 'index'], 'a_lisp_function_is_not_found_and_says_where_the_lisp_is',
     'a Lisp function is reported as Lisp, with where'),
    ('M16', f'{I}/src/lib.rs', '"{}: NOT FOUND by this index -- which is not the same as absent.",',
     '"{}: does not exist.",',
     ['-p', 'romacs-introspect', '--test', 'index'], 'an_unknown_name_states_its_coverage_and_never_claims_absence',
     'a refusal never asserts absence'),
    ('M17', f'{I}/src/lib.rs', '(true, true) => Some(Implementation::Both),', '(true, true) => Some(Implementation::CoreOnly),',
     ['-p', 'romacs-introspect', '--test', 'index'], 'car_is_implemented_by_both_runtimes',
     'VM2+core is reported as both (the prototype said VM2-only)'),
    ('M18', f'{I}/src/lib.rs', '.any(|d| d.observable || !observable_only),',
     '.any(|d| d.observable || !observable_only || true),',
     ['-p', 'romacs-introspect', '--test', 'index'], 'observable_divergent_primitives_include_maphash_and_report_what_was_excluded',
     'observable-only means observable rows'),
    ('M19', f'{I}/Cargo.toml', '[dependencies]\nserde_json.workspace = true\n',
     '[dependencies]\nserde_json.workspace = true\nromacs-chars = { path = "../romacs-chars" }\n',
     ['-p', 'romacs-introspect', '--test', 'boundary'], 'no_romacs_crate_is_a_dependency',
     'the crate stays runtime-free'),
    ('M20', f'{I}/src/registry.rs', '                None => d.core_added.push(n.to_string()),', '                None => {}',
     ['-p', 'romacs-introspect-live', '--test', 'snapshot'], 'a_snapshot_missing_one_primitive_is_reported',
     'the snapshot comparison is not vacuous'),
    ('M22', f'{L}/src/lib.rs', 'input_hash: Some(compiled_hash.to_owned()),',
     'input_hash: None,',
     ['-p', 'romacs-introspect-live', '--test', 'live_registry'], 'enumerate_records_provenance',
     'deterministic input fingerprint is recorded'),
    ('M22b', f'{L}/src/lib.rs', 'input_hash: Some(compiled_hash.to_owned()),',
     'input_hash: Some("0".repeat(64)),',
     ['-p', 'romacs-introspect-live', '--test', 'live_registry'], 'enumerate_records_provenance',
     'a plausible constant cannot impersonate the actual compiled input fingerprint'),
    ('M23', f'{I}/src/lib.rs', 'SetQuery::NoTestMentions => self.tests(n).files == 0,', 'SetQuery::NoTestMentions => self.tests(n).files <= 1,',
     ['-p', 'romacs-introspect', '--test', 'index'], 'untested_set_is_lexical_and_says_so',
     'untested means no scanned test names it'),
    ('M24', f'{I}/build/derive.rs', r'Regex::new(r#"(?:\(|#' + "')", r'Regex::new(r#"(?:\(|#' + "Q)",
     ['-p', 'romacs-introspect', '--test', 'derive'], 'test_mentions_are_calls_and_function_quotes',
     "#'name counts as a mention"),
    ('M25', f'{I}/build/derive.rs', '              | .*::.*\n', '',
     ['-p', 'romacs-introspect', '--test', 'derive'], 'ledger_rows_carry_symbols_observability_and_line',
     'a Rust path in a ledger row is not a Lisp symbol'),
    ('M26', f'{I}/src/lib.rs', '.then(|| "implementing fn span".to_string())', '.then(|| "implementing fn span".to_string()).filter(|_| false)',
     ['-p', 'romacs-introspect', '--test', 'index'], 'rust_fn_variant_and_span_map_back_to_lisp_names',
     'a fn span maps back to its Lisp name'),
    ('M27', f'{I}/src/lib.rs', '        let near = self.near_misses(name);', '        let near: Vec<String> = Vec::new();',
     ['-p', 'romacs-introspect', '--test', 'index'], 'a_typo_is_not_found_and_offers_the_near_miss',
     'a typo is offered its near miss'),
    ('M28', f'{L}/src/lib.rs', '            Ok(Err((false, e))) if e.contains("void or non-function") => {}',
     '            Ok(Err((false, e))) if e.contains("void or non-function") => found.push(name.clone()),',
     ['-p', 'romacs-introspect-live', '--test', 'live_registry'], 'vm2_probe_finds_names_bound_outside_the_table',
     'an unbound name is not reported as a VM2 function'),
    ('M29', f'{I}/build/derive.rs', r'format!(r"SubrSpec::(fixed\d+)\(', r'format!(r"SubrSpec::(fixedQ\d+)\(',
     ['-p', 'romacs-introspect', '--test', 'index'], 're_search_forward_is_in_both_runtimes_with_both_resolving_spans',
     'fixedN registrations keep their shape and implementing fn'),
    ('M30', f'{I}/build/derive.rs', '.is_some_and(|s| s.contains("Observable by a package"));',
     '.is_some_and(|s| s.contains("Observable by a package") && false);',
     ['-p', 'romacs-introspect', '--test', 'index'], 'maphash_carries_its_observable_divergence_with_gnu_beside_ours',
     "the ledger's observability classification reaches the answer"),
    ('M31', f'{I}/build/derive.rs', 'kind: "direct-slot",', 'kind: "primitive-table",',
     ['-p', 'romacs-introspect', '--test', 'index'], 'not_is_found_in_vm2_through_its_direct_slot',
     '`not` is attributed to its direct slot write'),
    ('M32', f'{I}/build/derive.rs', '"span": first_match_arm(&core, v), "candidates": []',
     '"span": Json::Null, "candidates": []',
     ['-p', 'romacs-introspect', '--test', 'index'], 'special_form_if_resolves_to_its_evaluator_handler',
     'a special form resolves to its evaluator handler'),
    ('M33', f'{I}/src/lib.rs', 'derived_at_unix: b["derived_at_unix"].as_u64().unwrap_or(0),',
     'derived_at_unix: b["derived_at_unix"].as_u64().map(|_| 0).unwrap_or(0),',
     ['-p', 'romacs-introspect', '--test', 'index'], 'every_answer_carries_provenance',
     'answers carry when they were derived'),
]


def run(m) -> tuple[str, str]:
    mid, rel, old, new, args, test, _why = m
    if args is None:
        return mid, 'SKIPPED (documented, not runnable)'
    path = ROOT / rel
    orig = path.read_text()
    olds, news = (old, new) if isinstance(old, list) else ([old], [new])
    mutated = orig
    for o, nw in zip(olds, news):
        n = mutated.count(o)
        if n != 1:
            return mid, f'ERROR: expected 1 occurrence of the target text in {rel}, found {n}: {o[:50]!r}'
        mutated = mutated.replace(o, nw, 1)
    try:
        path.write_text(mutated)
        p = subprocess.run(['cargo', 'test', *args, '--', test], cwd=ROOT, capture_output=True, text=True)
        out = p.stdout + p.stderr
    finally:
        path.write_text(orig)
    if _why.startswith('EQUIVALENT') and p.returncode == 0:
        return mid, f'SURVIVED as recorded (equivalent mutant): {test} passed'
    if p.returncode != 0 and f'test {test} ... FAILED' in out:
        return mid, f'KILLED by {test}'
    if p.returncode != 0:
        tail = [l for l in out.splitlines() if l.startswith('error')][:3]
        return mid, f'ERROR (did not reach the test): {tail}'
    return mid, f'SURVIVED: {test} passed'


def main() -> int:
    want = set(sys.argv[1:])
    bad = 0
    for m in MUTANTS:
        if want and m[0] not in want:
            continue
        mid, verdict = run(m)
        print(f'{mid}  {verdict}  -- {m[6]}', flush=True)
        bad += not (verdict.startswith('KILLED') or verdict.startswith('SKIPPED')
                    or verdict.startswith('SURVIVED as recorded'))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
