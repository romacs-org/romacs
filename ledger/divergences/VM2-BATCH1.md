## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-BATCH1 | `ROMACS_RUNTIME=vm2` batch session | `src/emacs.c:1696-1703` selects noninteractive execution; `src/keyboard.c:1170-1195` evaluates the preloaded top-level startup form | Opt-in VM2 evaluates only batch expressions, without preload, a populated `load-path`, preloaded `features` or `command-line-args`; unsupported options and interactive use exit 2, Lisp errors exit 255 with VM2 error text | **Package-observable staged subset:** preload functions are absent, require cannot discover libraries, and feature checks lack preloaded features. One runtime owns the session; the default core path is unchanged | <!-- symbols: load-path, features, command-line-args, require, featurep -->
