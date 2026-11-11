## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-DND1 | Dropping onto a frame | `pgtk-register-dnd-targets` gives GTK a frame's drop targets, and `pgtk-dnd.el` handles the `drag-n-drop` events that follow | Romacs's window takes no drops: `pgtk-register-dnd-targets` makes GNU's checks and keeps nothing, so `make-frame` runs `pgtk-dnd-init-frame` as GNU's. No `drag-n-drop` event arrives, so `pgtk-drop-finish` and `pgtk-update-drop-status`, called only while handling one, are undefined (ledger/defun-census.tsv) | **User-observable**: dragging a file onto the window does nothing | <!-- symbols: pgtk-register-dnd-targets -->
