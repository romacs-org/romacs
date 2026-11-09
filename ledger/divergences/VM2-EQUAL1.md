## Observable by a package

| ID | Area | GNU behavior | Romacs behavior |
| --- | --- | --- | --- |
| VM2-EQUAL1 | `equal` of closures and char-tables | `internal_equal` (`src/fns.c`) compares a closure's and a char-table's slots, so two separately made objects of the same content are `equal`. Romacs compares closures by code identity and captures, and char-tables by identity. | Romacs closures hold register code with per-site caches, which is no comparable slot vector, and `sxhash-equal` would have to agree. | Two closures from one site, or `eq` objects, compare as GNU's. `tests/byte_op_kernels.rs` keeps both cases listed. |
