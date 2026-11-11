## Native leaf batch boundaries

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-LEAF-VARDATA | `symbol-value` wrong-type payload in variable probe | `(symbol-value 2)` signals `(wrong-type-argument symbolp 2)`. | Signals `(wrong-type-argument listp 2)`. | Existing variable-access condition-data gap exposed by internal-make-var-non-special/0018; retained VALUE class pin, no invented oracle answer. | <!-- symbols: symbol-value -->
