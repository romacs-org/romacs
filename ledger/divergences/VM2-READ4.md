## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-READ4 | `read-from-string` nesting limit | GNU reads a 257-level parenthesized zero at offset 515. | VM2 signals `invalid-read-syntax` with `"reader nesting limit exceeded at byte 257"`: the source reader rejects depth >256; 256-level input succeeds at offset 513. Pin `read-from-string/0169` is `SIGNAL-VS-VALUE`; `/0173` is the successful lower-bound probe. Decimal integers past the fixnum range read as bignums, as GNU reads them. | **Package-observable resource bound:** no nesting expansion in this slice; fail rather than recurse without bounds. | <!-- symbols: read-from-string -->
