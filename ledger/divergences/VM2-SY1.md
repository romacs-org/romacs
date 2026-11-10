## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-SY1 | VM2 regexp-host syntax tables | complete syntax-descriptor char-tables, generic array access and parser flags | native regexp syntax classes and standard inheritance; opaque printing; generic array access rejects these tables. The separate SyntaxTable representation is also rejected by general char-table parent/range APIs; `(set-char-table-parent (standard-syntax-table) nil)` signals `char-table-p` instead of returning nil | enable syntax-dependent regexp matching without claiming the general syntax-table subsystem; **package-observable**, detailed live-GNU comparison in the VM2 ledger |  <!-- symbols: syntax-table, standard-syntax-table, copy-syntax-table, set-syntax-table, modify-syntax-entry, looking-at, string-match, re-search-forward, re-search-backward -->

## VM2-SY1 — Regexp-host syntax tables, not general char-tables

GNU `src/syntax.c` stores complete syntax descriptors in mutable char-tables;
`aref`, `aset`, parent manipulation, printer traversal, and parser flags can
observe those descriptors. VM2's native syntax tables currently retain only
regexp syntax classes and standard-table inheritance. `syntax-table`,
`standard-syntax-table`, `copy-syntax-table`, `set-syntax-table`, and
`modify-syntax-entry` are provided to configure regexp matching; they do not
claim the general char-table or syntax-parser protocols. They print as opaque
`#<syntax-table>` objects. Non-class descriptor flags do not affect this
regexp-only host, and generic array access rejects the table.

**Observable by a package: yes.** Fresh isolated byte-compiled GNU 32.0.50,
revision `b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b`, compared with compiled VM2:

```elisp
(car (aref (copy-syntax-table) 45))
;; GNU => 3
;; VM2 condition-case => (signal (wrong-type-argument integerp 45))
```

This is a bounded host prerequisite for the five regexp primitives, not a
claim to finish the syntax-table subsystem. Generic array-access diagnostics
inherit VM2's existing primitive error adapter. See `REGEXP.md` for supported
syntax-class and syntax-property matching. RX1's matcher-overflow-budget
behaviour is inherited unchanged from `romacs-regex-engine` and remains in
the root ledger.
