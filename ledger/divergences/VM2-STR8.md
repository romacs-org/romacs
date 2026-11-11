## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR8 | Non-Unicode JSON display | GNU native strings contain raw/extended characters | VM2 JSON values (`protocol.rs::typed_depth`, `lib.rs::display`) refuse RawString with named `not-unicode-string`; `eval_data.rs` prints raw bytes as octal but refuses non-Unicode extended printable output | JSON is Unicode-only; no invented Unicode scalar |
