## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR10 | Compiled string constant properties | GNU keeps properties on compiled string constants | VM2 `reader.rs::source_form` still drops properties when mapping UTF-8 strings to Form::String or unibyte to Form::Bytes | Existing loss explicitly retained per A3; P6 owns richer constants |
