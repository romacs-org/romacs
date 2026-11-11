## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-CUSTOM-FILE-GUARD | The function `custom-file` | A nil `custom-file`, or one naming the init, saves into the init (`lisp/cus-edit.el:5065`) | It never answers the init, by name, through a symbolic or hard link, or past a link with `..`, even one not written yet: it gives the default `custom.el`, and the name it checked. If that is the init too, nothing is saved: `custom-save-all` signals, and `customize-save-variable` sets the value for the session with GNU's "temporarily" message. Automatic saves stay out of a file the user wrote. `rlisp/romacs-saved-settings.el` |
