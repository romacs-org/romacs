# crates/vendor/tree-sitter
Romacs's own tree-sitter runtime: forked from the published 0.27.0 crate and
tailored to how Romacs parses, which is after every key, with a background
reparse, many buffers per grammar, and fontify by visible region.

## Boundary
- The goal is a runtime that is cheap and fast for Romacs, not a copy of
  upstream. Upstream's assumptions about its other users (Zed, Neovim, the
  CLI, wasm) carry no weight here: change, delete or replace anything whose
  cost Romacs pays and whose benefit only they get, up to rewriting the
  runtime whole.
- The one compatibility limit: grammars others built load and parse as they
  do with the published runtime: the ABI range of `parser.h` (in
  `crates/vendor/tree-sitter/include`), generated `parser.c` tables,
  external scanners, and the query language `treesit` modes ship.
- Callers are `crates/romacs-vm/src/treesit/` (`native.rs`, `ahead.rs`) and
  the grammars in `crates/romacs-grammars`; the Lisp face of it is GNU's
  `treesit` API plus `lisp/romacs-treesit-font-lock.el`.

## Contracts
- On error-free input, trees equal the published runtime's (E1), and a
  reparse equals a fresh parse of the same text (E2). `tools/treesit-fuzz`
  checks both for every grammar Romacs ships, after each of its edits. On
  input with errors, today's runtime and the published one both break E2
  through history-dependent recovery. Stage P has two goals: E2 on any
  input, which `--e2-everywhere` checks, and each top-level form parsing
  from its own text alone, which P's own tests check.
- Work saved is the measure: a change states the parse or query work it
  removes on a real file (bytes re-lexed per key, instructions per reparse)
  against the runtime before it.
- `ROMACS.md` lists every change from the published crate, one commit each,
  with its reason and its measurement.

## Tests
- `tools/treesit-fuzz` whenever this crate, a change listed in `ROMACS.md`,
  or a grammar pin in `crates/romacs-grammars` changes.
- `tools/treesit-query-diff` for query compiler and cursor changes. Run it
  as described in `tools/lib/treesit-query/README.md`.
- `python3 tools/dev-loop romacs-vm --profile gate -E 'test(treesit)'` for
  the callers.

## Decisions
- 2026-11-18: this crate is Romacs's, tailored to Romacs; the old aim
  of shrinking back to the published crate is dropped. A change worth
  offering upstream may still be offered by a person (upstream bars
  agent-opened pull requests), but nothing waits on it.
- 2026-11-23 (owner): we own this runtime, and the only restriction is
  compatibility with third-party grammars (the Boundary's limit). The
  published runtime is an oracle on error-free input only.
- 2026-11-23: the end state is a Rust runtime, built in slices T0 (fuzz),
  Q, L and P, each deleting its C counterpart.
- 2026-11-23 (owner): the 11-15 rule permits the Rust parser stage once the
  Rust query stage preserves query behavior and fontify performance and
  removes the C query engine.
- 2026-11-23: Q met "preserves query behavior and fontify performance".
  Whole-scroll instructions fell 0.9% for C and 1.0% for Rust. Python
  fontify calls used 0.03% fewer instructions. Python's focused replay at
  +0.10% is within that reading. Stage P may start.
- The unsafe census and smell counts still leave `crates/vendor/` out; code
  written here for Romacs is held to the workspace's rules all the same.
