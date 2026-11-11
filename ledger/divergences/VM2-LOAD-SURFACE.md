## Ordered ERT measurement

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-LOAD-SURFACE | Remaining evaluator/byte-compiler surface | GNU readevalloop is also used by eval-buffer and eval-region; byte compilation uses its own top-level environment, not runtime sibling evaluation. | Source load now shallow-expands and sequentially evaluates resulting progn children, resolving former VM2-RECORD-EAGER. The public eval_source compiler API still compiles each complete read form; eval-buffer, eval-region and .elc input remain unavailable. The loader uses a 128-expansion/native-stack guard. | Loader repair is not a claim to implement GNU byte compilation or the missing buffer evaluators. GNU source-load and compiled witnesses deliberately differ; see TOPLEVEL-PROGN.md. | <!-- symbols: load eval-buffer eval-region -->
