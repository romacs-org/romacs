## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-CUSTOM-LOAD | Saved settings at startup | Nothing loads `custom-file`: an init that wants it says `(load custom-file)`. `user-init-file` names the default init even when there is none, so a first session can save | A session that loads an init gives `custom-file` the value `custom.el` in `user-emacs-directory`, unless something set it, and loads that file after Romacs's defaults and before the init: a saved value replaces a default and the init has the last word. `user-init-file` names the init whether or not it exists, as GNU's does. -q and -Q load nothing. A file that fails to load is reported and the init still runs. `lisp/session/romacs-saved-settings.el`, `harness/options_live.py` |
