## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-READPROP1 | Reader property keys and diagnostics | GNU allows general Lisp property keys and applies triples in order. | Property-string parsing supports symbol-keyed plists and replacement ranges. Non-symbol keys and atom/dotted plists can be refused; incomplete-triple validation and buffer line/column error details remain partial. | A property's key is its symbol (VM2-FR5-3): any symbol reads as its own key, an uninterned one included, and an object that is no symbol is refused, never normalized. | <!-- symbols: read, read-from-string -->
