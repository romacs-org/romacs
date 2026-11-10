## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-EV8 | Temporary buffers remain unnamed, collectable internal allocations | GNU `with-temp-buffer` creates a named buffer then kills it on unwind | VM2 lowers `with-temp-buffer` to private `#:vm2-temp-buffer`, restores current buffer on unwind, and lets GC reclaim unretained temporary buffers. `generate-new-buffer` is supplied only by GNU's loaded `subr.el`, which composes the native `generate-new-buffer-name` and `get-buffer-create`; a bare VM does not define it. Ordinary named-buffer operations accept ASCII unibyte names as the same names as text strings; non-ASCII unibyte names signal an explicit unsupported error | Preserves existing 100,000-temporary-buffer GC regression without retaining every temporary as a named live buffer; no kill-buffer protocol is claimed | <!-- symbols: with-temp-buffer, generate-new-buffer, get-buffer -->
