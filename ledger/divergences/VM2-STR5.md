## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR5 | Non-Unicode interning | GNU interns raw/extended character names | VM2 `intern` signals `not-unicode-string` through `Leaf::string_text` (`src/prims/widen.rs`, `src/prim.rs`) | UTF-8 symbol-name storage boundary; no lossy conversion |
