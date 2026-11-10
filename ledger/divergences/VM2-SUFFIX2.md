## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-SUFFIX2 | `get-load-suffixes` lazy traversal and membership | GNU 32.0.50 returns nil with nil `load-suffixes` even when `load-file-rep-suffixes` is circular. Compression membership signals `(wrong-type-argument listp 7)` for atom 7, signals with the original list for an unmatched dotted tail, and returns immediately on a matching cyclic/dotted member. | VM2 now traverses representation suffixes only for reached load suffixes, and tests membership incrementally with malformed-list rejection and cycle detection. | No intended package-visible difference; repairs the former eager circular-list error, silently accepted malformed compression lists, and failure to short-circuit a matching cycle. VM2-SUFFIX1's safe rejection of GNU's crashing representation element remains unchanged. | <!-- symbols: get-load-suffixes -->
