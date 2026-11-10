## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-BUFNAME-RAW1 | Buffer names with raw bytes | A buffer name is any Lisp string; `(get-buffer-create "\\351x.txt")` names the buffer with the raw byte, which `buffer-name`, `get-buffer`, `rename-buffer` and `generate-new-buffer-name` round-trip and the mode line and `#<buffer ...>` show as `\\351` | VM2 keys buffers by a Rust string, so a raw-byte character of byte B stands in the model's name as the character U+10FF00+B (`crates/romacs-vm/src/buffer.rs` `buffer_name_text`/`buffer_name_value`); the name primitives convert at the Lisp boundary and the printer and mode line show `\\NNN`. A buffer deliberately named with a character in U+10FF80..U+10FFFF would read back as raw bytes | **Package-observable** only for names in that private-use range | <!-- symbols: get-buffer-create, get-buffer, buffer-name, rename-buffer, generate-new-buffer-name -->
