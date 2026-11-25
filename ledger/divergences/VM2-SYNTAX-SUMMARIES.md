## Observable by a package: syntax summaries

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-SYNTAX-SUMMARIES | Long syntax scans | Every call scans each character from FROM to TO; `syntax-ppss` parses from states syntax.el caches, invalidated below each edit. | From FROM to TO over at least four leaves of a multibyte text, with no TARGETDEPTH, STOPBEFORE or COMMENTSTOP and, where `syntax-table` properties are honoured, propertized past TO, the scan reads only its two end leaves and composes kept summaries of the leaves between (`crates/romacs-vm/src/syntax_index.rs`, `syntax_summary.rs`); every value, point included, is the scanner's. Summaries are kept per syntax table, by a serial no other table takes (not its address), and per count of char-table writes, which every change of any char table's characters, default, parent or extras (`modify-syntax-entry`, `aset`, `set-char-table-range`, `set-char-table-parent`, `set-char-table-extra-slot`, `optimize-char-table`) advances, per `comment-end-can-be-escaped` and per `parse-sexp-lookup-properties`; a `syntax-table` property change drops those of the text it covers. A syntax descriptor changed in place (`setcar` on a `(CODE . MATCH)` cons a table or a `syntax-table` property holds) is not seen until some char table, or that property, changes, by long parses and by forward list scans of every count alike (a warm `(scan-lists POS 1 0)` crosses a character whose property cons was `setcar`ed as it read it before): descriptor conses are shared values (`string-to-syntax` returns one cons for every plain descriptor, as GNU's `syntax_code_object` does), and no GNU or bundled code mutates one. `syntax-ppss` is syntax.el's; its long parses from the states it caches are the ones served. `romacs-inhibit-syntax-summaries` scans every character; `romacs-syntax-summaries-check` scans as well and signals where they differ; `romacs-syntax-summaries-stats` shows what was served. | An edit near the top of a large file no longer costs a scan of all the text after it. Tests: `tests/syntax_summaries.rs` (against pinned GNU, with edits), the seeded property tests in `syntax_scan.rs`, `syntax_summary.rs` and `syntax_index.rs`. | <!-- symbols: parse-partial-sexp, syntax-ppss, scan-sexps, modify-syntax-entry -->

A forward `scan-lists` or `scan-sexps`, any COUNT, DEPTH and FROM, folds
the same index at leaf boundaries (`crates/romacs-vm/src/syntax_index/forward.rs`,
`syntax_summary/lists.rs`). A span folds on one of two summaries: an
excursion, keyed by the scanner's mode alone, where the scan's depth stays
at 1 or more across it (no count event, floor or last-good move); or an
exact entry, keyed by mode, depth and floor, which carries its count events
and relative last-good position, where it makes fewer events than the scan
has left. Elsewhere the scanner reads on with the live count. Where
`syntax-table` properties are honoured, only text wholly below
`syntax-propertize--done` folds: the scan stops for
`internal--syntax-propertize` where GNU's does, with GNU's arguments, and
folds again after each call up to the new frontier. A leaf's summary is made
when a scan reaches it below the frontier, by the one scan that reads it;
the first scan after an edit reads plainly where the frontier lies inside
a leaf. A scan that ends within its first 1,024 steps never consults the
index, and the partition into leaves is cut, and its cuts checked, only
where scans reach. Values, errors and the
order and arguments of every stop for Lisp are the scanner's.

The index keeps summaries for up to four contexts (table, escaped comment
ends, property lookup), the least recently used replaced; a switch to a kept
context forgets nothing, and each context loses its summaries when the
process's char-table write count moves. Every context checks the leaf ends
it uses with its own table.

`romacs-syntax-summaries-stats` counts forward list scans per run (a scan
stopped for Lisp counts again when it goes on) in `:list-served` and
`:list-declined` (`short`, `properties` for the frontier inside a leaf,
`inhibited`, `quit`), the spans folded by kind in `:folds`, the
characters they held in `:skipped` and, of those, the ones folded on
summaries made before the call in `:reused`, and `:evictions`,
`:context-switches`, `:context-evictions`, `:context-invalidations` and
`:unsafe-cuts`. Quit is asked on entry and after every fold, as GNU's
`maybe_quit` before a scan; a run a poll stops publishes nothing (its
summaries are not used again and its counts are dropped) and goes on, if
nothing quits, where it stopped.
The summary inhibition and checking options also apply to this path.
