## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| LISP-GUD-PREFIX | What gud.el puts on its global prefix, `gud-key-prefix` (C-x C-a) | Loading gud.el binds `gud-global-map` on the prefix by `global-set-key`, replacing whatever was bound there (`gud.el`). | `lisp/gnu/progmodes/gud.el`: when a keymap other than `gud-global-map` is already on the prefix and `gud-global-map` has no parent, that keymap becomes `gud-global-map`'s parent before `gud-global-map` is bound, so the agents' prefix map (`romacs-agents-map`, whose keys are plain letters) stays reachable under gud's control keys. `test/romacs/lisp/romacs-agent-keys-tests.el` | A key gud binds wins as in GNU; a key it does not bind reaches the map that was there (`keymap-parent` of `gud-global-map` is that map, not nil). Romacs binds its agents on C-x C-a, the global prefix gud also uses; GNU binds nothing else there. | <!-- symbols: gud-key-prefix, gud-global-map -->
