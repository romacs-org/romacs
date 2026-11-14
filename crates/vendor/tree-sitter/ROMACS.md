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

None yet.

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
