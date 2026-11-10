## Observable by a package: isolates and frozen values

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-ISOLATE-CLOSURES | Functions sent to a worker | GNU has no boundary: any function object is callable anywhere. | `isolate-spawn` accepts a symbol (the worker calls its own definition, autoloading as needed) or a lambda form, a list the worker evaluates, with free variables supplied by `:bindings`; a closure object signals `(error "Cannot send closure" FN)`. | Closure export (literal identity, symbol relocation, macro and compiler dependencies, captures) is unresolved; design note §3.3, M6. | <!-- symbols: isolate-spawn -->
