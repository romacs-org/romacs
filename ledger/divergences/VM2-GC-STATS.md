## VM2 explicit garbage collection

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-GC-STATS | Missing interval pool | `alloc.c:6075–6098` returns nine rows, including `(intervals SIZE USED FREE)`. | `garbage-collect` runs the existing moving collector at its traced engine boundary and returns the other eight rows, in GNU order. No `intervals` row is returned. | `(assq 'intervals (garbage-collect))` is nil: properties use Rust containers, not an interval pool; inventing interval sizes/counts would be false. Pinned GNU shape is recorded in `crates/romacs-vm/tools/primh/evidence/garbage-collect.json`. | <!-- symbols: garbage-collect -->
