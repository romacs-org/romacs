## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-LOADNAME | File names of a compiled load | `load-file-name`, `load-true-file-name`, the `load-history` entry, `symbol-file` and the `after-load-functions` argument name the `.elc`. | They name the source `x.el`: there is no `x.elc` on disk, and naming one would point code at a file that does not exist. | An `eval-after-load` or `after-load-alist` key written with an explicit `.elc`, or an `after-load-functions` hook testing for `.elc`, fires in GNU and not here. In `gnu-lisp/` and `rlisp/` no key names `.elc`, and every one of the 14 sites matching `\\.elc` against a file name accepts `.el` too. | <!-- symbols: load-file-name, load-true-file-name, load-history, symbol-file, after-load-functions, eval-after-load -->
