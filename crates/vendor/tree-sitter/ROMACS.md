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

- `refactor(treesit): remove wasm runtime support` removes the unused Wasm
  store, build paths and bindings. Measurement: none (pure deletion).

- `refactor(treesit): remove parser logging and dot graphs` deletes unused
  diagnostics and their bindings. The scanner serialization buffer and
  grammar ABI logging callback remain. Measurement: none (pure deletion).

- `refactor(treesit): remove non-UTF-8 parsing` deletes unused UTF-16 and
  custom decoding paths. The C input layout still carries the encoding and
  decode slots. Measurement: none (pure deletion).

- `refactor(treesit): remove dead scanner error flow` deletes the flag and
  exits used by Wasm scanner failures. Measurement: none (pure deletion).

## Checking it

`tools/treesit-fuzz` builds this runtime and the published one with the
grammars Romacs ships, and compares their trees after every one of a few
hundred seeded typing-like edits per file; any difference fails it. Run it
whenever this crate, its patches, or a grammar pin in
`crates/romacs-grammars` change.

Licence: MIT, `LICENSE` here.
