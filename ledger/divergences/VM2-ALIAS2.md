## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-ALIAS2 | `defalias` optional DOCSTRING | A non-nil third argument replaces the symbol's `function-documentation` property (`src/data.c:1021-1022`) | VM2 accepts a third argument but never stores it (`crates/romacs-vm/src/bootstrap.rs:48,67-79`). After `(defalias 'audit-f 'car "audit-doc")`, `(get 'audit-f 'function-documentation)` remains nil, or retains a pre-existing property | **Silent, package-observable metadata loss:** alias-specific help is missing or stale although assignment succeeds. `(documentation-property 'audit-f 'function-documentation)` returns `"audit-doc"` on GNU; VM2 currently lacks that callable query as well. Explicit `(put 'audit-f 'function-documentation "audit-doc")` stores the property; this does not implement the missing query | <!-- symbols: defalias, function-documentation, documentation-property -->
