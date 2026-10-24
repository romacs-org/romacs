# Properties and markers semantic oracles

Inputs use the existing `(case NAME eval FORM)` typed gnudiff transport. Each
FORM is quoted, self-contained, and returns finite data (never buffer, marker,
overlay or closure objects). Run all numbered `.el` files through
`crates/romacs-gnudiff/lisp/gnudiff.el` with GNU `-Q --batch`; `expected.tsv`
is its verbatim `gnudiff-1` output, including arguments and results.

Coverage: insertion inheritance matrix; propertized insertion and value identity;
modification/no-op hooks, undo and relative ticks; plist order; directional
property boundaries and limits; read-only errors and inhibition; marker editing,
lifetimes and undo/redo; D1 GNU plist identity observation; M6 string semantics;
overlay/text-property composition. Font-lock digest investigation is separate.

The inheritance matrix includes whole-buffer and interior runs, all four insert
variants, nil/t/list stickiness, default-nonsticky and category fallback at BOB,
run boundaries, interior and EOB. Relative ticks avoid process startup dependence.
Large observation collections are vectors to bound the recursive transport depth;
intermediate interval snapshots are copied before subsequent plist mutations.

Record without a Rust build:

```sh
/usr/bin/emacs -Q --batch -l crates/romacs-gnudiff/lisp/gnudiff.el \
  test/gnudiff/props/0[1-9]-*.el test/gnudiff/props/10-*.el \
  > test/gnudiff/props/expected.tsv
```

GNU records a real modification tick/hook cycle even for `set-text-properties`
with an equal plist in this fixture. Do not replace that observed result with
the design's blanket no-op claim for all property mutators.

## Explicit blocked assertions

* D1: `08-plist-identity.el` records GNU identity and destructive mutation only.
  The intended Romacs shared-canonical-plist divergence cannot be asserted by a
  GNU equality oracle; it needs a separate Romacs-specific assertion at P3.
* M6: `09-string-runs.el` retains 20,000 match results and records string runs for
  match-string, propertize, concat and a cross-boundary substring. No available
  Lisp API attributes per-string property allocation, so the <=8-byte accounting
  assertion remains blocked; GC totals/RSS are not a substitute.

These inputs are exercised without a Rust build by `bench/props-oracle.py`.
Supply `--romacs`, `--gnu`, `--xdisp` and `--json` inside an admitted execution
slot. The runner verifies the startup image, rechecks GNU against the recording,
and compares Romacs. Existing baseline mismatches return nonzero; they are not
silently treated as accepted divergences.

`expected.tsv` records files 01–10. `expected-fontlock.tsv` separately records
`11-fontlock.el`, including the incomplete control/meta escape reader reproducer,
a small Elisp font-lock reproducer, Org, and full GNU xdisp.c C-mode font-lock.
Recording file 11 requires `CORE_BENCH_XDISP`; its exact input/output hashes are
in `oracle-fontlock.json`. Keep the file-11 recording separate from `expected.tsv`.
