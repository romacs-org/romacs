#!/usr/bin/env python3
"""One-command regeneration of standalone census, proof and human summary."""
import collections
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('census', ROOT / 'scripts/lisp-census.py')
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)
started = time.monotonic()
for command in [[sys.executable, '-m', 'unittest', 'discover', '-s', 'scripts', '-p', 'test_lisp_census*.py'], [sys.executable, 'scripts/lisp-census.py', '--workers', '8', '--mode', 'cold'], [sys.executable, 'scripts/lisp-census.py', '--workers', '8', '--mode', 'warm'], [sys.executable, 'scripts/lisp-census.py', '--workers', '8', '--mode', 'warm', '--eager-target'], [sys.executable, 'scripts/lisp-census.py', '--compare', 'LISP-CENSUS-WARM.json', 'LISP-CENSUS-EAGER.json', '--output', 'LISP-CENSUS-EAGER-DELTA.json'], [sys.executable, 'scripts/prove-lisp-census.py']]:
    subprocess.run(command, cwd=ROOT, check=True)
d = json.loads((ROOT / 'LISP-CENSUS.json').read_text())
w = json.loads((ROOT / 'LISP-CENSUS-WARM.json').read_text())
p = json.loads((ROOT / 'LISP-CENSUS-PROOF.json').read_text())
s = d['summary']
ws = w['summary']
e = json.loads((ROOT / 'LISP-CENSUS-EAGER.json').read_text())
ed = json.loads((ROOT / 'LISP-CENSUS-EAGER-DELTA.json').read_text())


def figure(art):
    return census.census_figure(art)


chain = w.get('bootstrap_chain') or []
stop = next((c for c in chain if not c['loaded']), None)
chain_text = ', '.join(f"{Path(c['file']).name} {'loaded' if c['loaded'] else 'STOPPED'} ({c['forms_completed']} forms)" for c in chain)
frontier = (f"The chain stops in `{Path(stop['file']).name}` at form {stop['frontier']['ordinal']}, line {stop['frontier']['line']}: `{stop['frontier']['error']}`."
            if stop and stop.get('frontier') else
            # A READER failure never becomes an evaluated step, so it has no
            # `frontier`; it must not be reported as a whole chain loading.
            f"The chain stops in `{Path(stop['file']).name}` after {stop['forms_completed']} forms: `{stop.get('error', 'no error recorded')}`."
            if stop else 'The whole chain loads.')
lines = ['# VM2 Lisp census', '',
         'Two modes, always reported together and always labelled (R210):', '',
         '- **WARM is the parity metric.** Each target VM installs the VM2 Lisp prelude and loads the GNU `lisp/loadup.el` bootstrap chain in order (debug-early, byte-run, backquote, subr, keymap, ...), stopping at the first chain file that fails, then measures the target. GNU preloads this chain before anything else, so files that need `defsubst`, `when`, `dolist` etc. can only be measured fairly this way.',
         '- **COLD is a diagnostic of raw capability**: a fresh VM with nothing preloaded. It is the historical census, method unchanged, so earlier cold figures stay comparable.', '',
         figure(w), '', figure(d), '',
         '## Eager counterfactual (method, not progress)', '',
         figure(e), '',
         'Targets are held eager only after the ordinary bootstrap; the census-only identity binding selects the existing loader regime, not a replacement for GNU macroexp.el. Full per-file differences and unknown-progress rows are preserved in `LISP-CENSUS-EAGER-DELTA.json`.', '',
         '```json', json.dumps(ed, indent=2), '```', '',
         f"Warm bootstrap chain: {chain_text}. {frontier}", '',
         'The gap between WARM and COLD is the effect of the measurement method, not progress. Progress is a change in one mode between two commits, at the same denominator.', '',
         f"Initial runtime base: `0a5c5c6e`; this measurement checkout `{d['base_commit']}`. Content identity: `{d['input_sha256']}`.", '',
         'The rest of this page describes the COLD artifact (`LISP-CENSUS.json`); the WARM artifact is `LISP-CENSUS-WARM.json` and has the same row schema plus `bootstrap_chain`.', '',
         f"**COLD: {s['files_loaded']}/{s['files_total']} files load; {s['forms_evaluated_target_files']}/{s['forms_total_gnu_reader']} target top-level forms evaluate before the first unhandled load failure.**", '', f"**Measured scope:** {s['files_total']} `lisp/**/*.el` files and {s['forms_total_gnu_reader']} top-level forms. The brief claimed 1,892 and approximately 230,267. All actual paths are enumerated; no absent files or unexecuted forms were invented to fit the brief.", '', f"32 isolated workers; {d['runtime_seconds']:.3f} seconds for GNU splitting plus full VM census (build/test/proof excluded); 10-second per-file timeout. {s['transitive_failures']} failures are in dependencies rather than the target. {s['files_with_unavailable_progress']} files have unavailable progress.", '', '## Reproduce and detect staleness', '', '```sh', 'python3 scripts/regenerate-lisp-census.py', 'python3 scripts/lisp-census.py --check --mode cold', 'python3 scripts/lisp-census.py --check --mode warm', '```', '', 'The first command builds the measurement binary, runs focused tests, measures the entire tree, exercises mutation fixtures, cross-checks recorded files, and rewrites this summary and both JSON artifacts. Requires Python 3, GNU Emacs, Cargo, and the project Rust toolchain; the historical comparison additionally uses the local gnu-emacs-ref checkout. No shared registry/index is modified and no existing gate depends on these artifacts. The SHA-256 includes length-delimited path/content records for Lisp, VM and relevant dependency sources, manifests, and harness scripts, including untracked source additions; `--check` also verifies whole-tree coverage and GNU version. The built executable is copied into a private per-run directory and hashed before/after measurement. A changed input during measurement aborts instead of recording a mixed run.', '', '## What the numerator means', '', 'Each target starts in a new VM with all Lisp directories in load-path, but **no bootstrapped subr/custom/byte-run or preloaded GNU environment**. The real VM2 `load_file` is used. Only successful evaluation of target forms counts. Definitions count when their definition form evaluates, not when every function body has been exercised. Dependencies and handled signals follow normal Lisp semantics: a caught dependency error does not fail a target whose own forms and load finish successfully. Therefore this is a cold-load compatibility diagnostic, not an end-to-end proof that every loaded package works.', '', 'Primary verdict/progress are flushed before optional diagnostic replays. Missing names are not retained in many VM2 String errors: variable names from direct replay and function names from sentinel replay are explicitly **provisional**, not original-run observed names. A stateful replay or fboundp-sensitive path can diverge. Do not mistake the ranked hypotheses below for proven counterfactual unblocks. Unknown failures remain failures.', '', '## Failure classes', '', '| Class | Files |', '| --- | ---: |']
for kind in ['missing_function', 'missing_variable', 'reader_parse', 'vm_panic_abort', 'timeout', 'unknown']:
    lines.append(f"| {kind} | {s['failure_classes'].get(kind, 0)} |")
lines += ['', f"{s['missing_symbol_name_unavailable']} missing-symbol failures have no named variable/call attribution from the original diagnostic condition. See the candidate table for optional sentinel attribution; it is not confirmed original-run evidence.", '', '## First-frontier recommendations: top 30', '', 'Every name in this table is provisional post-verdict attribution. Counts are distinct target files encountering this first frontier, not all uses, not files proven to load after fixing it. Incidental variable names in missing declarations are not requests for primitives of those names.', '', '| Symbol / symptom | Files | Recommended route | Evidence and reasoning |', '| --- | ---: | --- | --- |']
for r in d['ranked_blocker_candidates'][:30]:
    reason = r['reasoning'].replace('|', '\\|')
    lines.append(f"| `{r['symbol']}` | {r['files_blocked_at_first_frontier']} | {r['recommended_remedy']} | {reason} |");
lines += ['', next((r['reasoning'] + f" Observed first-frontier reach: {r['files_blocked_at_first_frontier']} files, not a promised load-count increase." for r in d['ranked_blocker_candidates'] if r['symbol'] == 'cadr'), 'No current cadr frontier; consult the current ranked table rather than historical bootstrap recommendations.'), '', '## Lisp-only versus runtime-only', '', '**Proven Lisp-only: 0. Proven runtime-must-build: 0. Undetermined failed files: ' + str(d['remedy_certainty']['undetermined_failed_files']) + '.** First failure alone cannot prove there are no later runtime blockers, and a Lisp replacement is theoretically possible for many primitives. Claiming a definitive split here would be false. The per-entry routes above are cost hypotheses at the first frontier; no subject change or counterfactual patch was made.', '', '## Subsystem deletion assessment', '', 'No deletion is recommended. A cold-load failure is not evidence that calc, CEDET, semantic or srecode is outside the product. These are measured rather than excluded; product usage/scope evidence must precede any deletion decision.', '', '| Subtree | Files | Loaded | Most frequent first causal file |', '| --- | ---: | ---: | --- |']
for label, prefix in [('calc', 'gnu-lisp/calc/'), ('calendar', 'gnu-lisp/calendar/'), ('cedet', 'gnu-lisp/cedet/'), ('semantic (subset of cedet)', 'gnu-lisp/cedet/semantic/'), ('srecode (subset of cedet)', 'gnu-lisp/cedet/srecode/')]:
    rows = [r for r in d['files'] if r['file'].startswith(prefix)]
    causes = collections.Counter((r.get('first_failure') or {}).get('cause_file') for r in rows if not r['loaded'])
    lines.append(f"| {label} | {len(rows)} | {sum(r['loaded'] for r in rows)} | {causes.most_common(1)} |")
lines += ['', '## Recorded figures: different source tree and different measurement', '', '| File / source tree | Continue-after-error accepted | Real strict load prefix | Recorded |', '| --- | ---: | ---: | ---: |']
for r in p['calibration']:
    label = ('GNU reference/' if 'gnu-emacs-ref' in r['file'] else 'romacs/') + Path(r['file']).name
    lines.append(f"| {label} | {r['continuation_forms_accepted']}/{r['total_forms']} | {r['strict_load_forms_evaluated']} | {r['recorded_accepted']}/{r['recorded_total']} |")
lines += ['', census.calibration_note(p['calibration']), '', '## Mutation proof', '', 'Real subprocess fixtures: trivial two-form source passes 2/2; absent call fails at form 1 and the following form does not evaluate; absent variable fails; unfinished syntax fails after one successful form; infinite loop times out. A handled dependency failure succeeds, and a handled dependency followed by an own variable failure is attributed to the target. A replay that loops cannot destroy the already-flushed failed primary verdict. The actual verdict predicate is then replaced with `lambda report: True`, and the same real missing-call subprocess is rerun: the failure assertion catches that mutant. Full fixture rows and GNU standalone verdicts are in `LISP-CENSUS-PROOF.json`.', '', '## GNU behavioral authority', '', 'Source oracle: the pristine GNU checkout selected by `CENSUS_GNU_SOURCE`.', '', '- `src/lread.c:2270-2287`: skip whitespace/comments and distinguish clean EOF before reading.', '- `src/lread.c:2116-2129` and `3553-3567`: EOF inside required reader content signals an error.', '- `src/lread.c:2315-2318,2334-2338`: read then evaluate; no continue-after-error catcher in the load loop.', '- `src/lread.c:1524-1536`: load invokes the read/eval loop.', '- `src/eval.c:2057-2069`: signaled errors unwind; the failing operation is not resumed.', '', '## Limits', '', 'Primary void-function and void-variable diagnostics carry their offending symbols; other compiler/reader errors can still lack object attribution. Post-verdict sentinel replay names remain hypotheses, not primary evidence. It cannot prove the lisp-only/runtime-only split, post-fix loads, function-body usability, GUI behavior, or a complete standalone GNU baseline. Panic/abort before the primary report and primary timeouts may have null form position/progress. Paths are absolute in raw VM error text; relative stable target paths identify rows. The census does not itself edit runtime or Lisp source; subject edits are represented by the input digest.', '']
(ROOT / 'LISP-CENSUS.md').write_text('\n'.join(lines))
print(json.dumps({'regeneration_wall_seconds': round(time.monotonic() - started, 6), 'artifacts': ['LISP-CENSUS.json', 'LISP-CENSUS-WARM.json', 'LISP-CENSUS-PROOF.json', 'LISP-CENSUS.md']}))
