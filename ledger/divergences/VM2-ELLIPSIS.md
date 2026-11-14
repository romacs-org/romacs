## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-ELLIPSIS | Truncation mark in GUI sessions | `truncate-string-ellipsis` nil, so `…` wherever the frame can display it | `romacs-gui-startup` sets `truncate-string-ellipsis` to three dots, so the function `truncate-string-ellipsis` returns them, and Romacs's own truncations read it (`lisp/romacs/romacs-gui-loadup.el`): its Lisp never writes a literal `…` or fixed dots to cut text or to end a label in a column or the status bar, but the function's value (a popup's entries, the agent chat's workers, Jabber's sync status, gnosis's header line). Batch keeps GNU's value | **User-observable**: truncated text ends in `...`; an init may set the variable | Iosevka paints `…` two cells wide where `string-width` counts one, which shifted every column after it | <!-- symbols: romacs-gui-startup truncate-string-ellipsis -->
