## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR6 | Non-Unicode file/process/buffer names | GNU accepts its full character strings at these boundaries (subject to platform coding) | VM2 rejects raw/extended strings with `not-unicode-string`: `source_string` in `loader.rs` serves load/file names and bootstrap paths; `buffer.rs::string_bytes` serves file/insert paths; `eval_buffer.rs::eval_buffer_name` serves buffer names; `productivity.rs::process_lines` serves process arguments | UTF-8 host/text boundary; named refusal, not Latin-1 substitution |
