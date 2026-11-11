# Romacs

Romacs is a Lisp machine emulator written in Rust, with Emacs Lisp as its
runtime, and with first-class support for agents. People and agents should be
able to inspect, program, and reshape the whole machine through Lisp. Editing
is one application of the machine.

Rust emulates the machine: object memory and collection, the evaluator, loading,
processes, and the display and input host. Elisp is its runtime: GNU's Lisp
corpus and Romacs's own Lisp run on it as programs, and Lisp should carry
commands, policy, composition, and customization wherever practical. Broad GNU
Elisp compatibility provides useful programs and semantics; it does not require
copying GNU Emacs's internal architecture.

Romacs is a GPU-rendered GUI program, plus `--batch` for noninteractive runs.
It has no TTY frontend: `romacs -nw` and GNU's `emacs -nw` or `--terminal`
display are not supported and are not planned. Don't design, test, or
measure for a text-terminal display. Romacs *contains* a terminal emulator
(`romacs-term`, which keeps Alacritty's behaviour); it is not itself a terminal
program. Pinned GNU may be run `-nw` as an oracle when no GUI comparison is
possible; say so where you do.

## Development

- Optimize for useful capabilities, easy modification, and a short feedback loop.
- A build or fix request authorizes focused edits, checks, review, and a local
  commit. Questions, explanations, and reviews remain read-only unless changes
  are requested.
- In Lisp the user sees, use faces, not bare strings: key hints through
  `substitute-command-keys` or `help-key-binding`, mode-line and status text in
  named faces that inherit theme faces. A command that only makes sense in a mode
  declares it (`(interactive nil some-mode)`), so `M-x` lists what applies.
- Make operations discoverable, queryable, and composable from Lisp. Agent
  interfaces should expose meaningful state, structured results, and clear errors.
- Prefer small functions and existing owners. Reuse GNU Lisp when it supplies the
  behavior; implement missing runtime substrate in its existing Rust module.
- Every new `unsafe` carries a `// SAFETY:` comment on the line directly above
  the keyword. `tools/lib/unsafe_census.py` (run by `tools/check-derived`) holds
  each crate's unsafe and undocumented sites to `ledger/unsafe-floors.json`. After
  deleting unsafe, run `python3 tools/lib/unsafe_census.py lower` and commit the
  lowered floors with the change. Raising a floor is a deliberate commit.
- `tools/lib/smells.py` (also run by `tools/check-derived`) counts the other
  smells: `allow(dead_code|unused*)`, TODO/FIXME, files over 1,500 lines, every
  rustc and Clippy pedantic warning by lint, the tree's top-level entries, and
  primitives declared outside `src/prims/` or through lib.rs's own enum. It
  holds a branch to its merge-base with the trunk (`ROMACS_BASE`, default
  `master`): a count above the merge-base's own fails. A drop needs no
  record, and a branch never lowers a floor. Adding a smell takes raising its
  floor in `ledger/smell-floors.json` by as much, and a new top-level entry takes
  listing it there: a deliberate commit with its reason. `smells.py check
  --absolute` holds the trunk to that file at a checkpoint, where `lower`
  records the drops. The file merges by its meaning, each side's raise added:
  `tools/derived setup` installs the merge driver once per repository, and
  `python3 tools/lib/smells.py resolve` does the same by hand. The same setup
  installs the driver that merges the host boundary allow-list entry by
  entry (`tools/lib/host_allow.py`).
- Follow the real consumer through its next failing form. Close a related
  dependency path end to end before expanding into unrelated primitives.
- Keep bootstrap regimes and compatibility adaptations explicit. Preserve promised
  representation boundaries and provide intentional introspection APIs.
- Treat GNU Lisp as a source of behavior, not of architecture. Keep the Elisp API
  real packages observe; replace an implementation whose design is platform or era
  baggage (tty/termcap, per-window-system backend dispatch, charset-era MULE, byte
  code in data files, startup rituals) with an API-compatible one in `lisp/romacs/`, and
  record it as a file named by its ID in `ledger/divergences/`. Keeping a
  mechanism because GNU has it is museum-keeping; deleting one because GNU has
  it is vandalism. Replace it when a better mechanism exists and real packages
  still work.
- A compatibility shim exists so old code does not error; it never preserves the
  defect beneath it.
- Keep one mechanism per job. A second mechanism for the same job is debt, even
  when both are new.
- When a mechanism moves to Rust, keep the policy users customize reachable from
  Lisp.
- Back structural and performance claims with measurements on the development
  machine.
- Judge the machine as a whole. Measure runtime and representation work on
  corpus workloads (loading GNU's Lisp and `lisp/romacs/`, the GNU ERT suites) and on
  general use (find-file and the minibuffer, project commands, dired, vc,
  search, help, shells, org and text editing), not on one user's configuration
  or one package.
- Keep working behavior and user data intact while improving the machine.
- Derived state is recomputed at batch boundaries, not per item. Mutations
  mark state dirty; expensive derivations (face realisation, display
  mirroring, layout, chrome refresh) run once per command, redisplay, or
  theme switch. Hot primitives index rather than scan growing collections.
  Every new collection-holding subsystem adds a scaling probe, one file under
  `crates/romacs-vm-gui/harness/scaling/probes/`. A probe over its exponent
  budget is fixed, or carries `:known` with a ticket. Raising its budget is
  not a fix.
- Attack cost in this order: complexity (never worse than GNU's; pick the
  textbook structure: interval trees, ropes, indexes, dependency graphs),
  then incremental work (recompute only what a change dirties, in dependency
  order), then laziness (visible region first), then parallelism.
- Parallelize work that is independent and larger than its dispatch and
  merge cost. Pure Rust work over independent data runs data-parallel inside
  its primitive, touching no Lisp state. Lisp work that can leave the main VM
  runs in worker VMs over buffer snapshots, its results applied at a batch
  boundary; the main VM stays interruptible.
- A parallel path proves itself twice: a scaling probe on large inputs, and
  perkey on ordinary ones (no per-key regression from dispatch).
- `doc/performance.org` collects the practices behind these rules: measuring
  slow keys, what runs on the main VM and in worker VMs, handing results
  over, and bounding the main VM's own work.

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
environment. Keep your HOME and Cargo environment. Never enter `nix develop`
for a toolchain: the flake's dev shell builds WPE WebKit from source, which
takes every core for hours and starves running sessions. If `cargo` is not on
PATH, ask the maintainer for the prepared environment. The acceptance gate
owns its isolated test homes and heavy-job scheduling. Use worktrees when parallel
writers need isolation; ordinary development does not need a lane registry.

Use the configured pinned GNU oracle and mirror. Retain an explicit
`ROMACS_GNU_SOURCE` when tests isolate HOME, and never substitute PATH's Emacs for
the pinned oracle.

The gates, small and whole for each side:

- Rust, small: `make gate-small`. Tests named after the Rust modules changed since
  master, or since `BASE=REV`, and the structural rules whatever changed (`STRUCTURAL`
  in `tools/gate`). A change it cannot map, such as Lisp or a crate root, runs
  everything, so give a branch of a branch its real base.
- Rust, whole: `make gate-full`, for the final combined candidate.
- Lisp, one module: `make check-MODULE`.
- Lisp, whole: `make check`.

The other commands:

- Focused runtime checks: `cargo test --profile gate --locked --offline -p romacs-vm --test TARGET`, optionally followed by `-- TEST_NAME --exact`.
- Fast development checks: `python3 tools/dev-loop PACKAGE --test TARGET`,
  optionally with `-E 'test(NAME)'`. Without `--test`, it selects library tests.
  `--list` shows selection without building; `--jobs` and `--threads` bound work.
  It preserves `CARGO_TARGET_DIR` and defaults to `dev-fast`. Use `--profile gate`
  for compute-heavy GNU loading and oracle tests, with the same focused selectors.
- Editable Lisp checks on a prebuilt host: `python3 tools/test-elisp`. Its
  `--test`, `--match`, file arguments, and `--core` option support smaller checks.
- Bundled packages' real ERT suites on a prebuilt host, as GNU's `make check`:
  `make check`, `make check-expensive`, `make check-all`, `make check-MODULE`
  (`make check-list` names the modules: a package like `gnosis`, or one library),
  with `SELECTOR=`, `TEST_FILES=`, `EMACS_TEST_TIMEOUT=` (seconds per file) and `JOBS=`.
  Each file runs in a fresh process with a private HOME. Set `ROMACS_TEST_BINARY`
  or use `CARGO_TARGET_DIR`'s gate build. `--logs DIR` on `tools/test-ert` keeps
  each file's output. Lisp edits take effect without a rebuild. `make check-runner`
  checks the runner itself against pinned GNU.
- ERT report/parser checks: `python3 -m unittest discover -s gnu-tests/tests -p test_ert_report.py`.
- Real GNU ERT execution: `python3 gnu-tests/run.py run --harness real --bootstrap ert --ert-adapter romacs-batch --bin BINARY --out OUTPUT FILE ...`.
- Scaling probes: `python3 crates/romacs-vm-gui/harness/scaling_live.py --binary BINARY --output OUTPUT`,
  optionally `--mode batch|gui`, `--only NAME ...`, and `--record`. `--record`
  writes call graphs for probes over budget; give it a frame-pointer build. The
  gate runs the batch probes (`romacs-vm-gui` test `scaling`).
- Derived artifacts, source/floor validation and the Python unit tests: `python3 tools/check-derived`.
  It runs every tracked `test_*.py`/`*_test.py` that no Rust test owns (`tools/lib/python_tests.py`).
  It can regenerate stale output; inspect and stage only task-owned changes.
- Combined verification: `python3 tools/gate focused 'EXPR'`, then
  `python3 tools/gate full` for final acceptance. Inspect `--help` and select
  resource bounds for the current machine. Defaults keep outer VM and display
  execution serial, with a 34G heavy-job cap. Supply the pinned GNU executable
  with `--oracle` when the gate's isolated environment needs it. Slot tools come
  from this repository and are frozen alongside the gate for each invocation.
  Add `--staged` to verify the index snapshot while preserving unrelated
  unstaged and untracked work. The gate binds evidence to its frozen Git tree;
  a temporary commit or a clean checkout is not needed.

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
- GNU's tests served to reach parity; they now serve the project. Change or retire
  a test, a reference answer or a floor when the behavior it measures is deliberately
  replaced or a better test covers it, and record the reason. Fix missing dependencies
  and runtime behavior first: never use stubs, skipped load forms, swallowed errors,
  new exclusions, or weaker assertions to hide a failure. Document intentional
  adaptations and remaining compatibility limits.

## Working safely

- Inspect Git identity, status, and relevant diffs before editing. Clear ambient
  `GIT_DIR`, `GIT_WORK_TREE`, and `GIT_INDEX_FILE` overrides before Git operations.
- Keep one writer per checkout. Give independent writers isolated worktrees and
  reviewers an exact candidate with read-only authority.
- Preserve unrelated edits, live sessions, and personal data. Use isolated test
  environments; restore your environment before signing commits.
- Keep plans, logs, receipts, and temporary experiments outside the source tree.
- Finish verified, reviewed work with focused signed local commits. Stage only
  owned paths. Push, publication, activation, service restarts, and real mail
  require explicit authorization.
- Report what changed, what actually ran, its result, and any remaining limit.
