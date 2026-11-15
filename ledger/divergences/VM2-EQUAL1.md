## Observable by a package

| ID | Area | GNU behavior | Romacs behavior |
| --- | --- | --- | --- |
| VM2-EQUAL1 | `equal` of char-tables | `internal_equal` (`src/fns.c`) compares a char-table's slots, so two separately made char-tables of the same content are `equal`. Romacs compares char-tables by identity. | A Romacs char-table is a native range structure with flat views and subdivision history, not GNU's slot vector, and `sxhash-equal` would have to agree. | `eq` char-tables compare as GNU's. `tests/byte_op_kernels.rs` keeps the case listed. Closures compare as GNU's: by their source key and captured values (`crates/romacs-vm/src/closure_key.rs`). |
