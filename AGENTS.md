# Romacs

Romacs is being built as a malleable Lisp machine with first-class support for
agents. People and agents should be able to inspect, program, and reshape their
environment through Lisp. Editing is one application of the machine.

Rust provides the runtime and platform machinery. Lisp should carry commands,
policy, composition, and customization wherever practical. Broad GNU Elisp
compatibility provides useful programs and semantics; it does not require
copying GNU Emacs's internal architecture.

## Development

- Optimize for useful capabilities, easy modification, and a short feedback loop.
- A build or fix request authorizes focused edits, checks, review, and a local
  commit. Questions, explanations, and reviews remain read-only unless changes
  are requested.
- Make operations discoverable, queryable, and composable from Lisp. Agent
  interfaces should expose meaningful state, structured results, and clear errors.
- Prefer small functions and existing owners. Reuse GNU Lisp when it supplies the
  behavior; implement missing runtime substrate in its existing Rust module.
- Follow the real consumer through its next failing form. Close a related
  dependency path end to end before expanding into unrelated primitives.
- Keep bootstrap regimes and compatibility adaptations explicit. Preserve promised
  representation boundaries and provide intentional introspection APIs.
- Treat GNU Lisp as a source of behavior, not of architecture. Keep the Elisp API
  real packages observe; replace an implementation whose design is platform or era
  baggage (tty/termcap, per-window-system backend dispatch, charset-era MULE, byte
  code in data files, startup rituals) with an API-compatible one in `rlisp/`, and
  record it in DIVERGENCES.md. Keeping a mechanism because GNU has it is
  museum-keeping; deleting one because GNU has it is vandalism. Replace it when a
  better mechanism exists and real packages still work.
- A compatibility shim exists so old code does not error; it never preserves the
  defect beneath it.
- Keep one mechanism per job. A second mechanism for the same job is debt, even
  when both are new.
- When a mechanism moves to Rust, keep the policy users customize reachable from
  Lisp.
- Back structural and performance claims with measurements on the owner's
  hardware.
- Keep working behavior and user data intact while improving the machine.

## Testing loop

1. Define the observable behavior and reproduce the failure at the boundary that
   owns it. Use a focused Rust test for a runtime defect, actual Lisp execution for
   a language behavior, and an isolated native journey for a UI change.
2. During edits, run the smallest relevant target or test selector. Build only the
   required host, then reuse its binary for Lisp-only edits. Do not rebuild the workspace
   or run the full GNU corpus after every small change.
3. Test cohesive paths together. For ERT, an assertion must reach its handler,
   backtrace, printer, summary, and process status. A primitive test alone does not
   prove its caller works. Run affected existing callers before the expensive gate.
4. Broaden checks when the change crosses an ownership boundary, affects shared
   runtime behavior, or reveals another failure. GC, evaluation, loading, protocol,
   and lifecycle changes need evidence for the relevant interactions.
5. Before final verification, inspect the diff, regenerate affected derived
   artifacts, pass focused checks, and obtain fresh read-only review for nontrivial
   changes. Freeze the candidate before running the required full project gate.
6. Reserve the full gate for the final combined candidate. Rerun affected checks
   after a correction; do not repeatedly gate unchanged inputs or rerun failures
   until a lucky pass appears. Documentation-only changes need reference and
   consistency checks, not a runtime rebuild.

## Existing commands

Run commands directly from the repository root in the prepared project
environment. Keep the owner's HOME and Cargo environment. The acceptance gate
owns its isolated test homes and heavy-job scheduling. Use worktrees when parallel
writers need isolation; ordinary development does not need a lane registry.

Use the configured pinned GNU oracle and mirror. Retain an explicit
`ROMACS_GNU_SOURCE` when tests isolate HOME, and never substitute PATH's Emacs for
the pinned oracle.

- Focused runtime checks: `cargo test --profile gate --locked --offline -p romacs-vm --test TARGET`, optionally followed by `-- TEST_NAME --exact`.
- Fast development checks: `python3 scripts/dev-loop PACKAGE --test TARGET`,
  optionally with `-E 'test(NAME)'`. Without `--test`, it selects library tests.
  `--list` shows selection without building; `--jobs` and `--threads` bound work.
  It preserves `CARGO_TARGET_DIR` and defaults to `dev-fast`. Use `--profile gate`
  for compute-heavy GNU loading and oracle tests, with the same focused selectors.
- Editable Lisp checks on a prebuilt host: `python3 scripts/test-elisp`. Its
  `--test`, `--match`, file arguments, and `--core` option support smaller checks.
- ERT report/parser checks: `python3 -m unittest discover -s gnu-tests/tests -p test_ert_report.py`.
- Real GNU ERT execution: `python3 gnu-tests/run.py run --harness real --bootstrap ert --ert-adapter romacs-batch --bin BINARY --out OUTPUT FILE ...`.
- Derived artifacts and source/floor validation: `python3 scripts/check-derived`.
  It can regenerate stale output; inspect and stage only task-owned changes.
- Combined verification: `python3 scripts/gate focused 'EXPR'`, then
  `python3 scripts/gate full` for final acceptance. Inspect `--help` and select
  resource bounds for the current machine. Defaults keep outer VM and display
  execution serial, with a 34G heavy-job cap. Supply the pinned GNU executable
  with `--oracle` when the gate's isolated environment needs it. Slot tools come
  from this repository and are frozen alongside the gate for each invocation.
  Add `--staged` to verify the index snapshot while preserving unrelated
  unstaged and untracked work. The gate binds evidence to its frozen Git tree;
  a temporary commit or a clean owner checkout is not needed.

`PACKAGE`, `TARGET`, `TEST_NAME`, `NAME`, `BINARY`, `OUTPUT`, `FILE`, and `EXPR` are placeholders.
Inspect the current commands and relevant tests rather than inventing another
runner. Census checks require gate-owned subject evidence and can correctly refuse
standalone Cargo execution without it.

## Test cost and evidence

- Budget the complete process fan-out: outer test concurrency multiplied by each
  test's internal workers, alongside display and compiler work. More workers can
  make cold startup slower and trigger timeouts.
- Keep display tests serial unless another configuration has been qualified.
- Before launching a broad check, know its selection and approximate cost. The GNU
  ratchet currently creates fresh VMs and repeats source bootstrap for each file;
  the full corpus is an acceptance check, not an edit loop.
- Investigate unexpectedly slow checks by separating build, bootstrap, and body
  execution. Measure before increasing deadlines or adding caching or worker
  machinery. Preserve isolation of mutable Lisp state.
- Reuse evidence only for the same relevant source, binary, environment, and check
  inputs. Record exact identities and the boundary actually exercised.
- Distinguish setup refusals, assertion failures, timeouts, and abnormal exits.
  A SIGKILL alone does not establish OOM or host disturbance.
- Keep selected, started, and terminal results separate. Zero selection,
  interruption, missing completion, and unexpected failure must not become success.
- Keep real GNU ERT, the reduced mini harness, and Romacs Lisp tests distinct.
  A reduced scoreboard is not proof that GNU ERT executed the bodies.
- Preserve GNU reference answers, imported test bodies, and protected floors.
  Fix missing dependencies and runtime behavior; do not use stubs, skipped load
  forms, swallowed errors, new exclusions, or weaker assertions to manufacture
  progress. Document intentional adaptations and remaining compatibility limits.

## Working safely

- Inspect Git identity, status, and relevant diffs before editing. Clear ambient
  `GIT_DIR`, `GIT_WORK_TREE`, and `GIT_INDEX_FILE` overrides before Git operations.
- Keep one writer per checkout. Give independent writers isolated worktrees and
  reviewers an exact candidate with read-only authority.
- Preserve unrelated edits, live sessions, and personal data. Use isolated test
  environments; restore the owner's environment before signing commits.
- Keep plans, logs, receipts, and temporary experiments outside the source tree.
- Finish verified, reviewed work with focused signed local commits. Stage only
  owned paths. Push, publication, activation, service restarts, and real mail
  require explicit authorization.
- Report what changed, what actually ran, its result, and any remaining limit.
