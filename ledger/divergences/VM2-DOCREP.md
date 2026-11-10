## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-DOCREP | Variable and face documentation of a compiled load | An `.elc` stores `variable-documentation` as a `(FILE . OFFSET)` reference that `documentation-property` resolves. | The documentation is an inline string in the plist; `documentation-property` gives the same text. | Code reading the raw property sees a string, not a reference. | <!-- symbols: documentation-property, get -->
