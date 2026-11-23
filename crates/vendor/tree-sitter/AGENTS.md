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
- On input without errors, every grammar's trees equal the published
  runtime's: there the grammar defines the answer, and that is what "grammars
  work" means. `tools/treesit-fuzz` checks it for the grammars Romacs ships.
- On input with errors (code being typed, broken code), recovery and tree
  shape are Romacs's to design for locality and cost. A tree may differ from
  the published runtime's only inside the top-level form under edit; forms
  the edit did not touch keep their trees.
- Work saved is the measure: a change states the parse or query work it
  removes on a real file (bytes re-lexed per key, instructions per reparse)
  against the runtime before it.
- `ROMACS.md` lists every change from the published crate, one commit each,
  with its reason and its measurement.

## Tests
- `tools/treesit-fuzz` whenever this crate, a change listed in `ROMACS.md`,
  or a grammar pin in `crates/romacs-grammars` changes.
- `python3 tools/dev-loop romacs-vm --profile gate -E 'test(treesit)'` for
  the callers.

## Decisions
- 2026-11-18: this crate is Romacs's, tailored to Romacs; the old aim
  of shrinking back to the published crate is dropped. A change worth
  offering upstream may still be offered by a person (upstream bars
  agent-opened pull requests), but nothing waits on it.
- 2026-11-23 (owner): we own this runtime, and the only restriction is
  compatibility with third-party grammars (the Boundary's limit). The
  published runtime is an oracle on error-free input only; form-local
  reparsing of the edited form is this runtime's design.
- The unsafe census and smell counts still leave `crates/vendor/` out; code
  written here for Romacs is held to the workspace's rules all the same.
