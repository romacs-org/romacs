## fns.c sequence access boundary

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-ELT3 | `elt` circular input printer control | Printer serializes the circular input `(1 2 1 2 . #2)` in the corpus control. | Refuses circular list printing without `print-circle`; actual elt circular access is tested successfully at maximum fixnum. | Existing printer gap, 1 PRINTER witness in `tools/primh/evidence/elt.json`; not an elt semantic alternative. | <!-- symbols: elt -->
