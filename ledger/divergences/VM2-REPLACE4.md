## VM2 replace-match

| # | Mechanism | GNU | VM2 | Why |
|---|---|---|---|---|
| VM2-REPLACE4 | `replace-match` register capacity | GNU retains allocated unused match registers, including startup search history, and reports nonexistent subexpressions inside that capacity; a buffer replacement's `\N` past that capacity signals invalid replacement text and one inside it inserts nothing. | VM2 uses its retained match-group vector length, so an unused SUBEXP can signal args-out-of-range earlier and its maximum differs, and a buffer replacement's `\N` past the match's groups signals where GNU's capacity may insert nothing. | Match-register allocation history is not modeled; successful groups and explicit nil groups retain their normal semantics. |
