## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR3 | Property-bearing string reader syntax | `src/lread.c:3907-3915` recognizes `#(...)`; `:3344-3365` installs each range's properties via `Fset_text_properties`. Live `#("abc" 0 2 (face bold))` reads successfully, yields `bold` at positions 0 and 1 and nil at 2; concat with `"d"` preserves the property range | VM2 source reader rejects the exact literal with `unsupported # reader syntax at byte 2`, before evaluation; nested concat rejects it at byte 10. [Live GNU/VM2 evidence](crates/romacs-vm/tools/primh/logs/str-substrate-reader.json) | **Package-observable unresolved reader/substrate boundary:** Lisp source or serialized data containing GNU's property-string notation cannot load. Existing mutable string-property primitives do not imply support for this reader notation; no parser implementation is included | <!-- symbols: read, read-from-string, concat, get-text-property -->
