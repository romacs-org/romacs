## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-EV1 | Ordinary mode/startup calls are data-model operations only | `buffer.c:Fforce_mode_line_update` returns ALL and marks live redisplay; GNU Lisp modes also initialize syntax, keymaps, hooks and frame menu state | VM2 `switch-to-buffer` selects/creates the current named buffer but ignores NORECORD/FORCE-SAME-WINDOW; nil buffer selection is rejected. `emacs-lisp-mode` sets buffer-local `major-mode` only. `menu-bar-mode` changes its boolean variable only; `menu-bar-lines` is a standalone 0 default and does not track a frame or mode. `force-mode-line-update` returns ALL without scheduling display work. `truncate-lines` is nil by default and local when set | Explicit headless subset: no frame/window/display mutation belongs to s124 | <!-- symbols: switch-to-buffer, emacs-lisp-mode, major-mode, menu-bar-mode, menu-bar-lines, force-mode-line-update, truncate-lines -->
