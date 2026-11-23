# Tree-sitter query differential

Run `python3 tools/treesit-query-diff --editor BINARY --out DIR` with the
prepared tree-sitter and Rust toolchain. Use a new scratch directory
outside the checkout and a host built with the checkout's bundled
grammars. In the editor, the extraction runs real mode setup, collects
the individual and combined font-lock queries at levels 0–4 and records
query calls during indentation. The recorder also loads the pinned
grammar packages' highlight and indent files, where supplied.

The runner builds the pinned C production engine with the same test
recorder, builds the current checkout, copies both test executables out,
and runs them over these hand-written samples and the extracted queries.
It checks compiled metadata, tree fingerprints and the exact match
sequence with capture IDs, node IDs, symbols, flags, byte ranges and
points. Reversed and sentinel bounds and subtree roots have separate
probes. Each grammar must contribute real mode queries, successful
compiles and captured matches. The only permitted differences are the
three separately checked reproductions in the runner. Shipped queries
are never exempted from comparison.

Use `CARGO_TARGET_DIR` and the build lock assigned to the lane. Everything
generated, including the C reference source, inputs, binaries and
records, stays under DIR or that target.

## Known C differences

The named probes use the pinned JSON grammar.

- `wildcard-negated-field` uses `{"k": 1}` and
  `(_ !key (number) @number) @parent`. C indexes the child and skips
  the parent's negated field. Rust checks the field and returns no match.
- `fourth-capture` uses the same input and `(number) @a @b @c @d`.
  C's fixed-size step stores three IDs. Rust emits all four in order.
- `wide-step-index` uses `true` with `(number) @n` repeated 32,768
  times followed by `(true) @end`. C's starting step index wraps and
  loses the match. Rust emits `end`.
