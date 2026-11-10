## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-EV5 | `locate-library` source-only lookup | GNU load/library search considers compiled/native/compressed suffixes and load policy | Searches load-path (or explicit PATH), .el then the literal name; honors NOSUFFIX. INTERACTIVE-CALL is accepted without UI/message effects. No .elc/.eln/compressed resolution | VM2 loads source; lookup does not pretend to find an executable compiled library | <!-- symbols: locate-library -->
