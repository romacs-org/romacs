## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-CHAR-TABLE-EQUAL | `equal` (and `sxhash-equal`) on two char-tables | `fns.c` internal_equal compares a char-table as a vector of its slots, sub-char-tables included, so a table whose character was set and then cleared keeps a sub-char-table a fresh table lacks and is not `equal` to it | Two tables are `equal` when their purpose, default, parent, extra slots and every character's value are (`crates/romacs-vm/src/hash.rs` char_tables_equal); the layout of the subdivisions is not compared | **Package-observable** only to code that compares tables differing in layout alone; Romacs's subdivisions are not GNU's, and what a table maps is what callers compare | <!-- symbols: equal, sxhash-equal -->
