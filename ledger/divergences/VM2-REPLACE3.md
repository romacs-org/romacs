## VM2 replace-match

| # | Mechanism | GNU | VM2 | Why |
|---|---|---|---|---|
| VM2-REPLACE3 | `replace-match` arity payload | GNU direct-call arity errors name the function as a symbol. | VM2's shared native arity dispatcher names the subr object; condition kind and argument count agree. | Shared dispatch representation remains observable. |
