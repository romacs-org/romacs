## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR2 | Mixed non-ASCII unibyte/multibyte `concat` and `format` | `src/fns.c:910-927,974-976` reserves expanded storage and converts unibyte input with `str_to_multibyte`. Live `(concat (unibyte-string 255) "λ")` returns a multibyte string with length 2, bytes 4, character codes 4194303 and 955; `(format "a %s" (unibyte-string 233))` returns `"a \351"` | VM2 converts a unibyte part of a multibyte result the same way (`prims/widen.rs` concat, `productivity.rs` format): each byte above 127 becomes the raw-byte character | None known | <!-- symbols: concat, format -->
