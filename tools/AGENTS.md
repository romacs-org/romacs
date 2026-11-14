# tools
The checks and runners of the tree: what a branch runs before it is submitted,
and what the merge queue holds it to.

## Boundary
- A check lives here as a command a person, a lane and the queue run alike; its
  rules and lists live in `tools/lib/`, each with its tests in `tools/tests/`.
- `tools/gate` runs its inner half from a frozen copy of itself outside the
  tree, so that half imports nothing from `tools/lib/`: what it needs there it
  runs as a command from the frozen sources.

## Contracts
- A finding is one line that says where, what, and after `; fix:` how.
  `tools/romacs-lint` compares findings without their line number and fix, so
  a finding that only moved is not new.
- A stage that could not run leaves its check incomplete, never passed:
  `tools/dev-check` exits 2 for it, 1 for a failure.
- A check holds a branch to its merge-base with the trunk, not to the trunk's
  tip: `tools/dev-check` takes the changed paths and the lint's base from it.

## Tests
- `python3 tools/check-derived` runs the test files under `tools/tests/` that
  no Rust test already runs (`tools/lib/python_tests.py` says which).
- One file: `python3 -m unittest tools/tests/test_dev_check.py`.

## Traps
- Nextest refuses a whole filter over one `binary_id` no package has. Only a
  package with an `all` test target is asked for it (`tools/gate`'s
  ALL_TARGETS).
- Git quotes a path that has a quote, a backslash or a byte past ASCII. Read
  its NUL-delimited output (`-z`) wherever a path is used again.

## Decisions
- The structural rules that run the compiler (`tools/gate`'s SLOW_RULES) are
  the queue's: `tools/dev-check` runs the rest, which take seconds once the
  test target is built.
