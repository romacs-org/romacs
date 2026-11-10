## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-DISPTAB1 | Per-window display tables | `set-window-display-table` installs a table the window's display uses before the buffer's and the standard one | VM2 stores the table on the window (`window-display-table`, `set-window-display-table`) for Lisp that reads it (dired's `dired--get-ellipsis-length`), but its display reads only `buffer-display-table` and `standard-display-table` | **Package-observable:** glyphs a window-local display table changes are not shown changed | <!-- symbols: window-display-table, set-window-display-table -->
