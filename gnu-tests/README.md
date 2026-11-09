# gnu-tests: GNU Emacs's own test suite as romacs's parity specification

`test/` here is GNU Emacs's `test/` directory at the oracle pin
`45b0f7699df`, imported byte for byte (`PIN.json`). **Never edit it.** Its
content hashes to git tree `cc4d1dbcc23b776e1eb39caef099b6a5ee26a3d1`, which is
GNU's own `45b0f7699df:test`; `run.py verify-pin` recomputes that hash from the
working files (no repository needed) and every run refuses to start, and
re-checks at the end, if it differs. Tests execute in a scratch copy, because
GNU's tests write beside themselves (`bytecomp-resources/*.elc`).

The milestone metric is **passed GNU deftests under real ERT**, not static
source forms. The native registration census observed 7,925 file-owned tests
in 510 eligible `test/lisp` files (not 7,925 passes). See
[the real ERT scoreboard](ERT-SCOREBOARD.md) for scope and failure semantics.
The older 6,987 / 565-file figure covers textual forms across the wider import,
not the 0.1.0 `test/lisp` runtime denominator.

## Run it

    python3 gnu-tests/run.py run src/fns-tests.el          # one file
    python3 gnu-tests/run.py run --all --jobs 16           # everything
    python3 gnu-tests/run.py verify-pin
    python3 gnu-tests/run.py check-ledger [--ledger FILE]

Output: `summary.json` (machine-readable), `results.jsonl` (one row per
deftest), `log/<file>.log` (the adapter's raw event stream). Exit 0 only if
every selected test passed; 1 otherwise; 2 invalid ledger or usage; 3 drifted
import or stale baseline; 4 infrastructure.

Real ERT: `run --harness real FILE`; native runner validation:
`run --harness real --engine gnu FILE`; mini-vs-real comparison inside the
pinned native GNU binary: `run --harness-fidelity FILE`. Set
`ROMACS_GNU_ORACLE` to the pinned executable; no PATH oracle is accepted by
these paths. Every real row identifies both `harness: gnu-ert` and its engine.
See [ERT-SCOREBOARD.md](ERT-SCOREBOARD.md).

## How a default mini-harness verdict is reached

1. GNU's reader splits the file into top-level forms (`tools/extract-forms.el`).
2. The adapter (`runner/`, binary `romacs-gnu-ert`) starts ONE fresh VM2 per
   file, installs VM2's prelude, loads the loadup bootstrap chain as far as it
   reaches (the WARM census chain), loads the harness, evaluates every form
   with `Vm::eval_source` (one failing form does not hide the rest), then runs
   each registered test and streams one JSON line per event.
3. `run.py` compares against GNU's verdicts recorded with real ERT
   (`baseline/`, re-recorded by `run.py baseline`), classifies, applies the
   ledger, writes the summary.

## The harness is REDUCED, and says so

Real `ert.el` cannot load on romacs yet: it requires cl-lib, debug, backtrace,
ewoc, find-func, pp, map and help-mode, and cl-lib is still being unblocked.
`harness/romacs-mini-ert.el` is a small stand-in for the part of ERT a test
file touches (`ert-deftest`, `should`, `should-not`, `should-error`,
`skip-unless`, `skip-when`, `ert-skip`, `ert-fail`, `ert-info`). Every summary
says `"is_gnu_ert": false`. It is never a Lisp feature for users.

To stop the harness from manufacturing passes, the baseline also runs **the
reduced harness inside GNU** (`tools/gnu-run.el mini`). A romacs verdict counts
only where that matches real ERT's outcome for the same test; elsewhere the test
is `not-yet-runnable` with `missing = "harness: ..."`. When cl-lib lands, the
target is to load GNU's real ert.el.

## Verdicts and categories

Verdict: `passed`, `failed`, `errored`, `skipped`, `not-yet-runnable`.
Category of every non-pass: `intentional-divergence` and `out-of-scope` (only
via `EXCLUSIONS.toml`), `not-yet-implemented` (names the missing symbol,
derived mechanically from the failure), `failing` (everything else), plus the
neutral `gnu-fails-too` / `gnu-skips-too`.

## The exclusion ledger

`EXCLUSIONS.toml` is the only way to set a test aside. A row must name one
deftest, an existing DIVERGENCES.md id, a symbol that entry covers and that
the test's source uses, and GNU's behaviour, ours and why it is intentional.
Any invalid row refuses the whole run before anything is measured; a row whose
test now passes refuses it too. There is no skip flag.

## The runner's own tests

`tests/test_run.py` (run by the gate via `runner/tests/python_contract.rs`)
and `tests/mutants.py`, which removes each protection in turn and requires the
targeted test to fail.

## The parity ratchet

The suite's passes are floors (`parity-ratchet.json` at the repo root,
`tools/lib/parity_ratchet.py`): per file, the exact deftests that pass. The gate
runs the whole suite in four shards (`runner/tests/parity_ratchet.rs`), and
the WARM and COLD Lisp census beside it (`crates/romacs-vm/tests/
parity_ratchet.rs`). A lost pass fails and names the test; a new pass fails
until recorded with `python3 tools/lib/parity_ratchet.py record` in the same
commit; a run that measured nothing is red. `tools/check-derived` refuses a
ratchet file that lowers a floor without a `--lower UNIT --because` record.
Quick local check: `python3 tools/lib/parity_ratchet.py check --tier fast`.

## The honest ratchet (schema 2)

A reduced-harness pass is not evidence until something shows it can fail.
`record` therefore stores, per passing member, a CLASS derived from
the run-local `MEMBER-EVIDENCE.json` and from what the measurement saw.
`tools/import-vacuity-evidence.py` generates it beside `VACUITY-VERDICT.json`
outside the source tree; `tools/gate` provides both paths to tests. Standalone
measurements set `ROMACS_VACUITY_RECEIPT` to the receipt, with the member file as
its sibling. Neither output is tracked. `member-evidence-policy.json` retains
the historical member names, classes and methods only; it contains no run
observations or provenance. The one sealed subject is regenerated from the
source-bound receipt.
`load_ratchet` and runtime `verify` re-derive every class and refuse a hand edit.
Static `verify --candidate` checks floor policy without replaying runtime evidence.

| Class | Protected (loss FAILS) | Counted | In the vacuous budget |
|---|---|---|---|
| GENUINE | yes | only if its file loaded with no failed top-level form | no |
| UNPROBEABLE (no or stale evidence) | yes | never | no |
| VACUOUS, WRONG-ERROR, WRONG-ERROR-DATA, HARNESS-SELF | no | never | yes |

- The **vacuous budget** may only decrease. A budget member that stops
  passing is `PROGRESS` (exit 3: record it), never a floor loss. A member
  enters the budget only through a deliberate `--lower` that moves it out of the
  protected floor in the same step.
- **Vacuous-pass rule** (run.py): the adapter loads
  `harness/romacs-mini-ert-audit.el` and reports each `should-error`
  satisfied by `void-function`, `void-variable` or `invalid-function` that the
  test did not ask for. Unless pinned GNU caught the same condition, the row
  is `failed`, category `vacuous-pass`. A pass with no audit is
  `unaudited-pass`, also not counted.
- **Pinned baselines only** (run.py, fail-closed): a baseline whose
  `gnu.sha256` is not `parity-reference.toml` `executable_sha256` is refused,
  naming the file and both hashes. Every baseline committed before this
  check was recorded by `/usr/bin/emacs-32.0.50`; re-recording them is a
  separate, deliberate act:
  `ROMACS_GNU_ORACLE=<pinned emacs> python3 gnu-tests/run.py baseline --all --force`.
- **Two labelled numbers**: `gnu_suite.passed` (REDUCED harness,
  `reduced_harness` says so) and `gnu_suite.real_ert` (GNU `ert.el`,
  unmodified). `gnu_suite.honest.counted` is the headline candidate for the
  reduced harness.
- Proof: `tools/tests/honest_ratchet_test.py` and `tests/test_run.py` (`Honesty`,
  end to end).

## Public baseline diagnostics

Historical `baseline/` JSON retains exact test membership, outcomes,
`as_expected` flags, should-error audits, condition names and data. Only the
recording user's home prefix inside `detail` and `load_error` diagnostics is
represented as `<recording-home>`. The remaining path suffix is retained; this
is a privacy redaction, not a newly recorded GNU answer or a change to any
expected test outcome. GNU test inputs and the imported test tree are unchanged.
