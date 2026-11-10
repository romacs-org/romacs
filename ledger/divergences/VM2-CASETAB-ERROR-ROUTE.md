## Observable by a package: case-table error routing

| ID | Surface | GNU behavior | VM2 behavior | Status |
|---|---|---|---|---|
| VM2-CASETAB-ERROR-ROUTE | Malformed non-nil standard-case-table extra slots during implicit scratch or legacy `with-temp-buffer` creation | Buffer initialization repairs the standard table through `set-standard-case-table`; invalid case-table structure signals `wrong-type-argument case-table-p`. | Normal `get-buffer-create` preserves the typed condition, but legacy implicit/temporary-buffer creation can erase it into a generic error. No successful substitute table is returned. | Existing creation error-routing boundary exposed by new case-table validation; not full casing support. | <!-- symbols: set-standard-case-table current-case-table -->
