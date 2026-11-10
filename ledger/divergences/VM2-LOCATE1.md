## VM2 format-message boundary

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-LOCATE1 | `locate-file-internal` file-name handlers and predicates | GNU `lread.c:openp` checks the suffixed candidate for a `file-exists-p` handler, then uses `file-readable-p` when selected; expansion can also dispatch `expand-file-name`. `fileio.c:find-file-name-handler` selects the latest-starting matching regexp (first on ties), respects handler `operations` and operation-specific inhibition, ignores case-fold-search, and preserves match data. | As GNU: a selected `file-exists-p` handler makes `file-readable-p` decide, and a function predicate's non-`dir-ok` verdict asks `file-directory-p`, both through Lisp, so their handlers run. | Regression coverage: `tests/locate_file_handlers.rs`; oracle pin `45b0f7699df36b3849edc3bdef37d4cb9e75b9ea`, GNU 32.0.50. | <!-- symbols: locate-file-internal, find-file-name-handler, inhibit-file-name-handlers, inhibit-file-name-operation -->

## VM2-LOCATE1 — Readable regular-file lookup subset

`locate-file-internal` implements GNU openp's search for every PREDICATE:
nil or t (readable), a natural-number access mode, and a function called on
each candidate (nil rejects it; a directory needs `dir-ok`). GNU defines `locate-file` itself in Lisp
(`lisp/files.el:1113`) and only `locate-file-internal` in C
(`src/lread.c:1590`), so the substrate provides the C entry point ONLY: a bare
VM has no `locate-file` until GNU's own `files.el` loads, and anything needing
the wrapper must load that Lisp rather than call a native stand-in.
The existing native file-name expansion/handler restrictions also apply:
nonempty `file-name-handler-alist` is refused rather than invoking handlers,
and `~user` and above-root expansion are unsupported. This lookup accepts
readable regular files, not GNU's additional non-directory device/FIFO cases.
These are unresolved compatibility gaps, not alternative GNU answers.
