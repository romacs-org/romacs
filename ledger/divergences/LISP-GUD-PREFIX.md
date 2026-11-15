## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| LISP-GUD-PREFIX | What gud.el puts on its global prefix, `gud-key-prefix` (C-x C-a) | Loading gud.el binds `gud-global-map` on the prefix by `global-set-key`, replacing whatever was bound there (`gud.el`). | `lisp/romacs/romacs-agent-ui.el`, after gud.el loads (an autoload `with-eval-after-load`): when `gud-global-map` has no parent, `romacs-agents-map` (whose keys are plain letters) becomes its parent, so the agents' keys stay reachable under gud's control keys on C-x C-a; gud.el itself is GNU's. `test/romacs/lisp/romacs-agent-keys-tests.el` | A key gud binds wins as in GNU; a key it does not bind reaches the map that was there (`keymap-parent` of `gud-global-map` is that map, not nil). Romacs binds its agents on C-x C-a, the global prefix gud also uses; GNU binds nothing else there. | <!-- symbols: gud-key-prefix, gud-global-map -->
