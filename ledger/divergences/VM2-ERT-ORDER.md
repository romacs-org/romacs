## Ordered ERT measurement

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-ERT-ORDER | MEASUREMENT REGIME: retained failed bindings prefix | GNU aborts a failed loadup dependency. | Explicit `--bootstrap ordered` retains only bindings.el failure at form 129, line 1225, `(void-function map-keymap)`, then loads easy-mmode after the macro definitions in its prefix. Other failures remain fatal. | Diagnostic progress measurement only, not an accepted bootstrap or STANDARD WARM figure; avoid compiling easy-mmode before bound-and-true-p exists. |
