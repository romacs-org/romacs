# Test tools

What runs for a change, and how. The root AGENTS.md's testing rules hold here;
this file says where each piece lives, so a tool gains no second copy of a rule.

## One map from a change to what must run

`tools/lib/impact.py` answers what a change can reach: the Rust tests (whole
packages, single test files), the live harnesses, the runtime Lisp for the Lisp
tier, and the structural rules. Its docstring states the rule by path. The
small gate (`tools/gate changed`, `make gate-small`), dev-check and the train's
tier 1 ask it: import `select(paths, root, metadata)`; shell callers run
`python3 -B tools/lib/impact.py select [--base REV] [PATH...]`, which prints the
same answer as JSON.

- A rule about what reaches what goes in `impact.py`, with a case in
  `tools/tests/test_impact.py`. No tool keeps its own path list.
- It reads the workspace graph from `cargo metadata --no-deps`; a new crate needs
  no entry.
- Within a crate's `src/` it selects the whole crate and its dependents: a
  module-level guess can miss a test, and a missed test lands a break.
- What it cannot place runs everything, and says which path did.
- `STRUCTURAL`, `SLOW_RULES`, `CRATE_RULES` and `HARNESSES` live there too.
  `tools/gate` forwards the first three, for callers that read them there.

## Test targets

`tools/lib/test_modules.py` folds a package's test files into one executable:
`all`, or `gnu` for a file that boots GNU's Lisp (it includes
`support/gnu_lisp.rs` or a file built on it), whose tests restore that boot from
a world. `crates/romacs-vm-test-build` applies the same rule when the tests
build. `targets` names what a check builds and runs; `equal` proves the folded
targets hold exactly their files' tests.

## Live harnesses

`crates/romacs-vm-gui/harness/NAME_live.py`, each taking `--binary B --output
DIR`, in a private headless compositor and HOME. A harness compares Romacs
with pinned GNU through the editors' own state where it can
(`parity_live.py`'s engines); it reads pixels only for what only pixels show.
Its verdict rests on work (instructions, paints), not wall time, which load
stretches. Which harnesses a change calls for is `impact.py`'s (`HARNESSES`,
`HARNESS_RULES`).

## The rest

- `tools/gate`: the acceptance gate (full, focused, changed).
- `tools/dev-check`: what would send a branch back, before it is submitted.
- `tools/check-derived`: derived artifacts, floors and the Python tests.
- `tools/test-ert`, `make check-MODULE`: bundled packages' ERT suites.
- `tools/dev-loop`: focused crate tests during edits.
