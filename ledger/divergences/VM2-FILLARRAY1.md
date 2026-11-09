## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-FILLARRAY1 | `fillarray` of a char-table | `Ffillarray` (`src/fns.c:3308`) stores ITEM in the 64 top-level slots and the default, but not in the table's ASCII cache, which `CHAR_TABLE_REF_ASCII` reads for characters 0-127: pinned GNU answers `(1 2)` for `(let ((ct (make-char-table 'foo))) (aset ct ?a 1) (fillarray ct 2) (list (aref ct ?a) (aref ct ?b)))`, and `(make-char-table 'foo 7)` filled with nil still reads 7 for ASCII | VM2 writes the t and nil ranges as `set-char-table-range` does (`crates/romacs-vm/src/prims/str.rs`), so every character, ASCII included, reads ITEM: `(2 2)`; other characters, the default and the parent match GNU (`tests/fillarray_char_table.rs`) | **Package-observable, only through GNU's stale cache:** code that set ASCII entries before filling the table sees the fill, not the old entries | <!-- symbols: fillarray, char-table -->
