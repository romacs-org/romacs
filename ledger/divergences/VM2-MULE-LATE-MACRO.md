## Ordered ERT measurement

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-MULE-LATE-MACRO | Early compiled push expander | GNU interpreted push can expand macroexp-let2 and gv-letplace once their definitions exist. | Ordered bootstrap compiles GNU subr.el push before those two macros exist; mule-util.el form 6 line 168 later signals invalid-function. Reloading only the unchanged real push definition after the prefix makes all 18 mule-util forms load in a separate diagnostic. | Same early-compilation ordering class as easy-mmode, not cured by the top-level progn boundary; no native Lisp stand-in added. | <!-- symbols: push macroexp-let2 gv-letplace -->
