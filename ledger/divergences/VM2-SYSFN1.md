## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-SYSFN1 | `gap-size`, `delete-terminal` with FORCE, `suspend-emacs`'s STUFFSTRING | `gap-size` is the current buffer's gap; `delete-terminal` with FORCE deletes even the sole terminal; `suspend-emacs` stuffs STUFFSTRING into the terminal's input | A buffer's text is a rope, so `gap-size` is 0. Romacs has one terminal, so `delete-terminal` signals GNU's sole-terminal error also with FORCE. `suspend-emacs` runs its hooks and stops the process group as GNU; there is no terminal input to stuff | **Package-observable:** `memory-report` counts no gap. | <!-- symbols: gap-size, delete-terminal, suspend-emacs -->
