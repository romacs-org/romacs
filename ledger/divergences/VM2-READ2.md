## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-READ2 | Reader token boundaries | GNU stops before hash, NBSP and vertical tab. | Vertical tab is now handled exactly; hash and NBSP still remain part of ordinary symbol tokens. Only read-from-string/0163 and /0164 remain VALUE pins. | Remaining bounded tokenization difference, not a printer gap. | <!-- symbols: read-from-string -->
