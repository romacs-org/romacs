# tools
The checks and runners of the tree: what a branch runs before it is submitted,
and what the merge queue holds it to.

## Boundary
- A check lives here as a command a person, a lane and the queue run alike; its
  rules and lists live in `tools/lib/`, each with its tests in `tools/tests/`.
- `tools/lib/impact.py` is the one map from changed paths to what must run: the
  Rust tests (whole packages, single test files), the live harnesses, the
  runtime Lisp for the Lisp tier, and the structural rules (`STRUCTURAL`,
  `SLOW_RULES`, `CRATE_RULES`, `HARNESSES`). `tools/gate`, `tools/dev-check` and
  the train's tier 1 ask it; `python3 -B tools/lib/impact.py plan` prints it for
  a shell. A rule about what reaches what goes there, with a case in
  `tools/tests/test_impact.py`; no tool keeps a path list of its own.
- `tools/lib/test_modules.py` folds a package's test files (romacs-vm's,
  romacs-vm-gui's: `PACKAGES`, `FOLDED`) into one executable, `all`, or in
  romacs-vm `gnu` for a file that boots GNU's Lisp; `targets PACKAGE` names
  what a check builds.
- `tools/gate` runs its inner half from a frozen copy of itself outside the
  tree, so that half imports nothing from `tools/lib/`.

## Contracts
- A finding is one line that says where, what, and after `; fix:` how.
  `tools/romacs-lint` compares findings without their line number and fix, so
  a finding that only moved is not new.
- A stage that could not run leaves its check incomplete, never passed:
  `tools/dev-check` exits 2 for it, 1 for a failure.
- A check holds a branch to its merge-base with the trunk, not to the trunk's
  tip.
- What `impact.py` cannot place runs everything, and it says which path did.
  Within a crate's sources it selects the whole crate and its dependents: a
  module-level guess can miss a test.

## Tests
- `python3 tools/check-derived` runs the test files under `tools/tests/` that
  no Rust test already runs (`tools/lib/python_tests.py` says which).
- One file: `python3 -m unittest tools/tests/test_dev_check.py`.

## Traps
- Nextest refuses a whole filter over one `binary_id` no package has. Only a
  package with folded test targets (`all`, `gnu`) is asked for them
  (`impact.test_file`).
- Git quotes a path that has a quote, a backslash or a byte past ASCII. Read
  its NUL-delimited output (`-z`) wherever a path is used again.

## Decisions
- The structural rules that run the compiler (`SLOW_RULES`) are the queue's:
  `tools/dev-check` runs the rest of what `impact.py` selects.
- A live harness's verdict rests on work (instructions, paints), not wall time,
  which load stretches.
