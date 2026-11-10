### Planned, not yet in effect

| mechanism | GNU | planned | observable by |
|---|---|---|---|
| charset **definition** | charsets definable, aliasable, prioritisable at runtime | fixed table built at startup; queries unchanged | a package calling `define-charset-internal`, `unify-charset`, `set-charset-priority` — none in the 7-package corpus, but CJK input methods do this |
