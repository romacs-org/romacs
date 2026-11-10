## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR7 | Non-Unicode buffer insertion and Unicode-only specifications | GNU buffers can hold full-range characters | VM2 refuses `not-unicode-string` in `buffer.rs` insert, `annotations.rs::annotation_string` (kill/yank), `annotation_primitives.rs` insert-before-markers, `command.rs` interactive specifications, `keymap.rs` kbd and `display.rs` name/text paths through `source_string`; `conditions.rs::message_string` also requires Unicode | Text rope and host UI remain Unicode-only |
