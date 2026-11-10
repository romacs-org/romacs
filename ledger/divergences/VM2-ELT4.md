## fns.c sequence access boundary

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-ELT4 | `elt` on the separate native syntax-table kind | `(elt (standard-syntax-table) 97)` returns a syntax descriptor cons, because GNU syntax tables are char-tables. | Refuses with `(wrong-type-argument sequencep TABLE)` rather than inventing a descriptor. This is NOT GNU-equivalent for that valid input. Ordinary CharTable objects are supported, but VM2's separate regexp-host SyntaxTable is not. | Inherited VM2-SY1 representation boundary; explicit refusal witness in elt corpus. Repair requires the out-of-scope syntax/char-table substrate, not an elt approximation. | <!-- symbols: elt -->
