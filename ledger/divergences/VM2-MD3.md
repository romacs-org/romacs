## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-MD3 | `set-match-data` negative end extraction | GNU `search.c` retains signed match offsets; after `(set-match-data '(2 -1))`, `(match-string 0 "abcd")` returns `"c"` through substring's negative end normalization. | VM2 stores offsets as unsigned internally; `match-data t` round-trips `(2 -1)`, but extraction rejects the wrapped end and signals `(args-out-of-range "abcd" 2 -1)`. `set-match-data/0507` is pinned `SIGNAL-VS-VALUE`, not a printer gap. | **Package-observable bounded regexp state:** signed match state across every consumer needs separate substrate work; this slice documents safe range rejection rather than claim extraction parity. | <!-- symbols: set-match-data, match-string -->
