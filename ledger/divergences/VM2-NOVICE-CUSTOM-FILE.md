## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-NOVICE-CUSTOM-FILE | `enable-command`, `disable-command` | Both append a `put` form to the init, or create `~/.emacs` when there is none (`lisp/novice.el:113`) | They write the same form to the file the function `custom-file` answers, opened by that literal name, where it survives Custom's saves. With no file to save to, the session only. `rlisp/romacs-saved-settings.el` |
