## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-ELLIPSIS | Truncation mark in GUI sessions | `truncate-string-ellipsis` nil, so `…` wherever the frame can display it | `romacs-gui-startup` sets `truncate-string-ellipsis` to three dots, and Romacs's own truncations read it (`rlisp/romacs-gui-loadup.el`). Batch keeps GNU's value | **User-observable**: truncated text ends in `...`; an init may set the variable | Iosevka paints `…` two cells wide where `string-width` counts one, which shifted every column after it | <!-- symbols: romacs-gui-startup truncate-string-ellipsis -->
