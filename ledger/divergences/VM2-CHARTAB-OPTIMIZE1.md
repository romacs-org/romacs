## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-CHARTAB-OPTIMIZE1 | `optimize-char-table` with a TEST function | `chartab.c:676` compares sub-table entries with `equal` (nil), `eq`, or by calling TEST for each pair. | nil, `equal` and `eq` collapse sub-tables exactly as GNU does (printed structure compared with pinned GNU); any other TEST signals `(error "VM2 optimize-char-table supports only the equal and eq tests")`. | Calling Lisp per comparison would need a T2 continuation through the recursive walk; GNU's preloaded Lisp uses only the default test. | <!-- symbols: optimize-char-table -->
