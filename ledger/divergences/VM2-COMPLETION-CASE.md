## Observable by a package: circular completion tables

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-COMPLETION-CASE | Completion case folding, string comparison and casing | GNU uses its current case tables, with nil entries meaning identity. | Original native tables retain Unicode fallback for nil entries above ASCII before GNU enrichment. Explicit non-nil entries override it; ASCII and replacement-table nil entries mean identity. | Preserve the native Unicode startup policy while respecting explicit ASCII edits and replacement tables. Range-crossing and replacement-table tests cover the boundary. | <!-- symbols: try-completion, all-completions, test-completion, compare-strings, upcase, downcase -->
