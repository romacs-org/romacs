# tree-sitter 0.27.0, as Romacs carries it

This directory is the `tree-sitter` runtime crate exactly as crates.io
publishes version 0.27.0 (checksum `2038684e…`, from upstream's `lib/` at
6070dbfe), without the registry's `.cargo-ok` and `Cargo.lock`, plus the
patches listed below. The workspace's `[patch.crates-io]` points
`tree-sitter = "=0.27.0"` here; `exclude` keeps it out of the workspace,
and the unsafe census and smell counts leave `crates/vendor/` to upstream.

## Why it exists

Romacs reparses after every key, and some of upstream's rules for reusing
the previous tree make a reparse redo far more than the edit needs. Each fix
lands here as its own commit, written as a contribution to upstream, so that
this copy can shrink back to the published crate.

## Patches

Each is its own commit in Romacs's history, written for upstream.

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
- `perf(query): find the progress offset only when the callback runs`
  (`src/query.c`). With a progress callback set, the cursor found the
  current node's start byte at every step of its walk, though the callback
  that reads it runs once in 100 steps. That cost a query 3%, enough that
  Romacs ran its queries without the callback and a query that matched
  nothing could not be stopped. The offset is now found just before the
  callback runs; what the callback sees is unchanged.

## Back to stock

When a published release holds every patch above, delete this directory and
the `[patch.crates-io]` entry, pin that release, and run `tools/treesit-fuzz`.

## Checking it

`tools/treesit-fuzz` builds this runtime and the published one with the
grammars Romacs ships, and compares their trees after every one of a few
hundred seeded typing-like edits per file; any difference fails it. Run it
whenever this crate, its patches, or a grammar pin in
`crates/romacs-grammars` change.

Licence: MIT, `LICENSE` here.
