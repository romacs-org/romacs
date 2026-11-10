## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-BUFNAME1 | Buffer-name string identity | GNU `buffer.c:1320` returns the stored Lisp string itself; live primh `buffer-name/0037` records `(ok t)` for repeated-call `eq`. | Stores an untraced Rust name and returns a fresh Lisp string; that row is pinned `VALUE` (`nil`). | **Package-observable identity gap:** avoid retaining a new moving Lisp value in this bounded buffer-model slice. Names and dead-buffer nil are otherwise real state. | <!-- symbols: buffer-name -->
