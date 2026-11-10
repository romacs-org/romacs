## Ordered ERT measurement

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-PROGN-DIAGNOSTIC | Additional failed-prefix diagnostics | GNU aborts a failed dependency load. | Independently measured ewoc after failed cl-preloaded and help-mode after failed faces retain those extra failed prefixes solely for attribution. A push-definition/subr reload is another independent diagnostic. | None is an accepted bootstrap, STANDARD WARM gain or real-ERT execution; strict adapter retains only VM2-ERT-ORDER. See TOPLEVEL-PROGN.md. |
