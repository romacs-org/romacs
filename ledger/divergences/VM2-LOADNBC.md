## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-LOADNBC | `no-byte-compile` beside an `.elc` | GNU loads an existing `.elc` even when its source now sets `no-byte-compile`. | Such a file loads as source: a compile refuses it, as `byte-compile-file` would. | Only an `.elc` left from before the cookie was added differs. | <!-- symbols: no-byte-compile, load -->
