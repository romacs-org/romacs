## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR9 | Compiled raw/extended string constants | GNU retains runtime-generated constants with full-range characters | VM2 `reader.rs::source_form` refuses multibyte RawString with `not-unicode-string: source constant (P6)`; normal UTF-8 and unibyte constants keep existing Form encodings | P6 needs source-cache codec owner; no cache skip or new Form variant in P1a |
