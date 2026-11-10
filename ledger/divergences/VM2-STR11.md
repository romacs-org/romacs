## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR11 | Raw string executable images | GNU dumped strings support full character range | VM2 `code_image.rs:54-60` existing String-only guard refuses RawString; mapped image strings remain UTF-8 multibyte, including ASCII (GNU ASCII literals are unibyte) | `clear-string` and `fillarray` additionally refuse mapped strings as immutable; image schema is out of lane; String::as_str remains infallible and this file is unchanged |
