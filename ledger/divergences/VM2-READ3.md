## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-READ3 | `read-from-string` special float syntax | GNU reads general signed decimal `N.NNe+INF` as infinity: `2.25e+INF` => `(1.0e+INF . 9)`, `-3.75e+INF` => `(-1.0e+INF . 10)`, both numeric. | VM2 reads these two tokens as symbols with offsets 9/10, not numbers; `numberp` returns nil for both. Pins `read-from-string/0166`, `/0167`, `/0174` are semantic `VALUE` differences, **not** floating-point printer differences. | **Package-observable bounded numeric reader:** its existing special-spelling subset is not GNU's general special-float grammar. | <!-- symbols: read-from-string -->
