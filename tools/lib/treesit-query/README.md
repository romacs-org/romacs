# Tree-sitter query differential

Run `python3 tools/treesit-query-diff run --out DIR --corpus-digest SHA256`
with the prepared toolchain, the lane's `CARGO_TARGET_DIR` and its build
lock. Stage the intended source before running. The runner archives that
tree and the pinned C revision under a new scratch directory outside the
checkout.

The runner builds and verifies the C-backed editor first. That editor
loads the archived subject's Lisp to collect actual mode queries. It
records individual and combined font-lock queries at levels 0–4 and
query calls during indentation. The pinned C lockfile supplies the
grammar packages' highlight and indent files, where available.

Before building the candidate, the runner writes `corpus.json`. The
manifest names the source tree, mode and grammar source digests, every
query and its digest, and every seeded edit, resulting text digest and
ordered range.

Both `run` and `compare` require the expected `--corpus-digest`, which
replaces the former `corpus.sha256` sidecar. The digest is the SHA-256 of
the manifest with `c_editor_sha256` omitted, serialized as compact JSON
with sorted keys, ASCII escapes and a trailing newline. The runner
checks the digest before and after each recorder runs. The completed
run's `receipt.json` records `c_editor_sha256` as provenance only.

The comparison requires one compile outcome for every manifest query and
one execution for each successful query at every scheduled seed, edit
and range. It compares metadata, tree fingerprints and complete ordered
captures, including node identity, symbol, flags, byte range and points.
The named probes check exact records for subtree and byte bounds and for
the three isolated C differences below. Only those probe records may
differ. The regular tests also prove that cancellation returns before a
no-match traversal exhausts the tree.

Use `tools/treesit-query-diff compare --manifest DIR/corpus.json
--corpus-digest SHA256 --reference DIR/oracle-c.records --candidate FILE`
to check a replacement record stream. Add `--expect-c` when comparing two
C arms. Both copied editors, the build logs and both original record
streams remain in DIR.

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
