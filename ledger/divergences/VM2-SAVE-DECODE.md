## Observable by a package: save batch (WIP, not accepted)

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-SAVE-DECODE | String coding conversion | GNU performs full codec conversion. | `encode-coding-string` and `decode-coding-string` support nil/no-conversion, UTF-8 and raw-text coding systems, aliases, EOL conversion, simple BOM handling, NOCOPY and last-coding-system-used. GNU ASCII fast paths are retained. Other conversion codecs, coding hooks, BOM-pair detection and buffer destinations explicitly refuse. | This is a bounded string-conversion domain, checked against pinned GNU in `tests/coding_strings.rs`; it does not establish general codec or file-coding parity. | <!-- symbols: encode-coding-string, decode-coding-string -->
