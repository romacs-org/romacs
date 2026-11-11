## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-TITLE | The default window title | `frame-title-format` and `icon-title-format` start as `(multiple-frames "%b" ("" "%b - GNU Emacs at " system-name))` (`xdisp.c:38853`) | The same format and API, in Romacs's own name: `"%b - Romacs at "`. Setting either variable works as GNU's | **User-observable**: the title bar names Romacs | <!-- symbols: frame-title-format, icon-title-format -->
