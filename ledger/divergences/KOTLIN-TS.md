## Observable by a package

| # | mechanism | GNU | Romacs | why |
|---|---|---|---|---|
| KOTLIN-TS | Kotlin editing | No bundled Kotlin major mode or Kotlin file associations | `kotlin-ts-mode` derives from `prog-mode` and uses the bundled Kotlin grammar for indentation, highlighting, definition movement and Imenu. Visiting `.kt` or `.kts` selects the mode when the grammar is available. | Provide Kotlin editing on a fresh install. Packages can observe the mode, file associations and parser. | <!-- symbols: kotlin-ts-mode, kotlin-ts-mode-maybe, syntax-propertize-widen -->

The mode enables `syntax-propertize-widen` so a narrowed edit preserves
the lexical context established outside the restriction. Syntax parsing
treats interpolation as part of the string. Unescaped nested quotes in
`${...}` can affect syntax state until the syntax engine supports
interpolation.
