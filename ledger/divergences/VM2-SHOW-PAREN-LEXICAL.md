## Observable by a package: lexical Show Paren in tree-sitter modes

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-SHOW-PAREN-LEXICAL | Default Show Paren provider in tree-sitter modes | `treesit-major-mode-setup` installs `treesit-show-paren-data` for modes that define a `list` thing. | Mode setup leaves Show Paren on the lexical matcher unless the mode or user supplies another provider. The lexical matcher does not pair multi-character delimiters such as HTML/XML tags, Ruby/Elixir `do`…`end`, or Lua `function`…`end` where a mode defines them as pairs. In the mode hook, use `(setq-local show-paren-data-function #'treesit-show-paren-data)` to opt back in. | Cursor movement can match character delimiters without querying the tree. Covered by `test/romacs/lisp/paren-treesit-tests.el`. | <!-- symbols: treesit-major-mode-setup, treesit-show-paren-data, show-paren-data-function -->
