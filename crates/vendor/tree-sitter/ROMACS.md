# tree-sitter 0.27.0, as Romacs carries it

This directory is the `tree-sitter` runtime crate exactly as crates.io
publishes version 0.27.0 (checksum `2038684e…`, from upstream's `lib/` at
6070dbfe), without the registry's `.cargo-ok` and `Cargo.lock`, plus the
patches listed below. The workspace's `[patch.crates-io]` points
`tree-sitter = "=0.27.0"` here; `exclude` keeps it out of the workspace,
and the unsafe census and smell counts leave `crates/vendor/` out.

## Why it exists

Romacs reparses after every key, and some of upstream's rules for reusing
the previous tree make a reparse redo far more than the edit needs. This
runtime is Romacs's own, tailored to that use (`AGENTS.md` here); each change
lands as its own commit, listed below with its reason and measurement.

## Patches

Each is its own commit in Romacs's history.

- `perf(parser): enumerate viable missing terminals from the grammar row`
  (`src/parser.c`, `src/language.c`, `src/language.h`). Recovery scans
  the grammar's existing entries once to collect shifts whose destination
  can reduce the current lookahead. The parser still tries those candidates
  in ascending symbol order and scans scalar lookups for larger grammars
  and nonempty default actions. An edit in GNU's `xdisp.c` needed 40,154
  terminal lookups to try 277 candidates. The scratch row retains
  those same 277 attempts and carries no state across recoveries.

- `fix(parser): reuse a word token that can be no keyword` (`src/parser.c`).
  A node whose first leaf is the keyword-capture token (Python's
  `identifier`) was reused only when its recorded entry state, the state of
  the stack it was reduced on, equalled the current one. That state differs
  whenever the statements before it were grouped otherwise, as repetition
  balancing does, so every top-level statement after an edit was lexed
  again: 11% of a 279 KB Python file per key. Only a word the keyword lexer
  matches can lex differently in another state; any other lexes as the word
  token wherever the lex mode is the same, which the check already requires.
  Such nodes are now reused whatever their entry state. A typed character's
  reparse there re-lexes 570 bytes instead of 31 KB (6.7 to 1.4 ms); trees
  equal the published runtime's after every edit `tools/treesit-fuzz` makes.
- `perf(treesit): Switch query compilation and matching to Rust` moves
  compilation and matching into the VM's `treesit/query`
  module and removes the 4,880-line C query engine and its bindings.
  `src/query_adapter.c` copies grammar facts and hidden cursor state and
  returns both node byte offsets in one call until the tree moves to Rust.
  Progress polling is in the
  Rust walk, including walks that produce no matches. The prior C patch
  computed the progress offset only at callback time and saved 3% of a
  query's work.
  On the query-only C, Rust and Python corpus, cold compile took
  86–90% of the C engine's instructions and warm matching took 87–98%.
  Page-scroll evaluator instruction ratios against the C parent were
  0.9912 for C, 0.9896 for Rust and 1.0037 for Python over five Python
  pairs, with identical faces. The live Python fontification calls used
  0.03% fewer instructions. The focused Python replay used 0.10% more.
  The engine checks negated fields on wildcard roots, keeps every capture
  attached to a step and uses wide step indices. The differential records
  the three C defects in isolated probes. See VM2-TREESIT-QUERY.

### L0

- `refactor(treesit): remove wasm runtime support` removes the unused Wasm
  store, build paths and bindings.

- `refactor(treesit): remove parser logging and dot graphs` deletes unused
  diagnostics and their bindings. The scanner serialization buffer and
  grammar ABI logging callback remain.

- `refactor(treesit): remove non-UTF-8 parsing` deletes unused UTF-16 and
  custom decoding paths. The C input layout still carries the encoding and
  decode slots.

- `refactor(treesit): remove dead scanner error flow` deletes the flag and
  exits used by Wasm scanner failures.

- `docs(treesit): record the native UTF-8 boundary` documents the runtime
  after removing its unused paths.

- `docs(treesit): record L0 instruction counts` records the cost of the
  combined deletion slice.

- `refactor(treesit): remove the remaining UTF-16 binding helper` drops
  the unused slice accessor and describes only the supported text input.

- `refactor(treesit): remove empty lexer logging` drops the no-op macro
  and its calls. The grammar ABI keeps the callable logging slot.

- `perf(treesit): restore inlining of subtree size` keeps
  `ts_subtree_total_size` inline at the hot reuse and stack sites where
  GCC emitted calls after L0.

The cost rig at `7a80f952a` compared L0 `8b5eb2504` to `fa9115480`
using Callgrind. Runtime lexing used 7.1–11.2% fewer instructions and
per-key lexer lifecycle ranged from 4.4% fewer to 0.62% more. Per-key
whole-parse instructions rose 3.19% on T0 `clean-large` and 3.17% on
T0 `wide`, and 2.98% and 2.96% on the same rows in release. Lex dispatch
and parse call counts were unchanged on those rows. The profiles and
disassembly attribute the increase chiefly to GCC emitting out-of-line
calls to `ts_subtree_total_size`, `reusable_node_descend` and the
`ts_stack_push.constprop` clone at hot sites where the reference inlined
the work.

The cost rig at `3bfc984ab` compared the inlining fix `ee12843e3` to
`fa9115480`. With the rope reader, per-key whole-parse instructions fell
0.24–2.36% across both profiles. T0 `clean-large` fell from 2,737,505,260
to 2,715,967,148 instructions and T0 `wide` fell from 2,754,222,182 to
2,732,581,672. Runtime lexing used 7.04–10.78% fewer instructions and
per-key lexer lifecycle ranged from 2.46% fewer to 0.37% more. Both rig
gates passed. The whole-tail reader also used fewer whole-parse
instructions on every per-key row in both profiles.

## Checking it

`tools/treesit-fuzz` builds this runtime and the published one with the
grammars Romacs ships, makes a few hundred seeded typing-like edits per file,
and after each one checks the two equalities `AGENTS.md` names, on every
state where either runtime's reparse or fresh parse has no error: E1, the
reparsed tree equals the published runtime's in the ranges that changed and
in every node and cursor getter Lisp can reach; E2, the reparsed tree
equals this runtime's own fresh parse. A difference in either fails it, as
does a worker that fails or whose output is missing or misnamed, a
self-test plant that produces anything but its own (check, cause) failure,
or, in a default run, a grammar of `crates/romacs-grammars` with no such
state after an edit. `tools/lib/treesit-fuzz/test_getters.py` holds the
worker's getters to the wrappers in `crates/romacs-vm/src/treesit/native.rs`,
and every other file of `crates/romacs-vm/src` but Q's query engine and
its differential test to not naming the runtime; it guards against honest omission, not against code
written to hide a call. On text with
errors this runtime, like the published one, does not hold E2 yet (a reparse
keeps some recovery a fresh parse makes otherwise, in GNU's C sources a few
nodes in some of the edits), so those are counted and fail only with
`--e2-everywhere`. Run it whenever this crate, its patches, or a grammar pin
in `crates/romacs-grammars` change.

Parse states are counted apart and do not fail it; GNU's `treesit` API does
not show them. One difference there is known: the word-token patch above
changes `ts_parser__can_reuse_first_leaf`, which the token cache also calls
(`src/parser.c`, `ts_parser__get_cached_token`), so a cached word keeps the
state it was lexed in. That gives other parse states than the published
runtime in the 11 grammars with a keyword-capture token (bash, C, C++, Go,
Java, JavaScript, Kotlin, Python, Rust, TypeScript, TSX); tree shape is the
same.

Licence: MIT, `LICENSE` here.
