## Observable by a package: lexical delimiter blinking

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-BLINK-LEXICAL | Matching a typed closing delimiter | `blink-matching-open` uses the major mode's `forward-sexp-function`. | Blink uses the syntax-table scanner with the same distance and mismatch checks. | Lexical matching avoids forcing a tree-sitter parse under the temporary blink restriction. Covered by `test/romacs/lisp/simple-blink-tests.el`. | <!-- symbols: blink-matching-open, forward-sexp-function -->
