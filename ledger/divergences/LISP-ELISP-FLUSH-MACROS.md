## Observable by a package: GNU Lisp written for the engine

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| LISP-ELISP-FLUSH-MACROS | Elisp buffers' faces after a load | elisp-mode.el's `elisp--font-lock-flush-elisp-buffers`, on `after-load-functions`, runs `font-lock-flush` in every Elisp buffer after every load, with a FIXME that a load defining no macro should not | It flushes only when the loaded file's `load-history` entry names a macro (`elisp--load-defined-macro-p`), or when it has no entry. **To a package:** an Elisp buffer is not flushed after a load that defined no macro; its faces, which name macros, could not have changed | The first `cl-some` loads cl-extra and help-mode, neither defining a macro, and refontified every Elisp buffer; `test/romacs/lisp/elisp-mode-flush-tests.el` pins both cases | <!-- symbols: elisp--font-lock-flush-elisp-buffers, elisp--load-defined-macro-p, font-lock-flush -->
