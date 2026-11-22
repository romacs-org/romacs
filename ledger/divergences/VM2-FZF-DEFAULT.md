## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-FZF-DEFAULT | Interactive completion style | Global styles are `(basic partial-completion emacs22)`. Category defaults can take precedence. Fido prefers `flex`. | Interactive `--loadup` sets the standard `completion-styles` to `(fzf)` and clears built-in category defaults before init. Fido uses the global styles and category defaults when fzf comes first. GNU and noninteractive Romacs retain their completion defaults. | One matching rule across prompts. Category defaults previously let a prefix match mask fzf results in buffer and other prompts. | <!-- symbols: completion-styles, completion-category-defaults, completion-category-overrides, romacs-completion--fido-fzf -->

Users and packages can still select other styles through
`completion-category-overrides`. File completion uses fzf within each
directory. Users who complete abbreviated directory components such as
`/u/l/b` can choose `partial-completion` for the file category.
