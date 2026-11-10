## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-READSAFE1 | Detached marker input | Pinned GNU crashes with SIGSEGV for `(read (make-marker))` (banked read.dead-marker). | Signals a safe error before reading. | Intentional safety divergence; a GNU crash is never a target behavior. | <!-- symbols: read, make-marker -->
