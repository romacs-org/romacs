## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-READPROP1 | Reader property keys and diagnostics | GNU allows general Lisp property keys and applies triples in order. | Property-string parsing supports symbol-keyed plists and replacement ranges. Non-symbol keys and atom/dotted plists can be refused; incomplete-triple validation and buffer line/column error details remain partial. | The existing property owner stores keys as names, not identity-bearing Lisp objects. No non-symbol key is silently normalized. | <!-- symbols: read, read-from-string -->
