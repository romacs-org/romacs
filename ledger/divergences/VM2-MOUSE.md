## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-MOUSE | The mouse's frame | `mouse-position` and `mouse-pixel-position` answer the frame under the pointer, a child frame included, in its coordinates (`mouse_position_hook`) | Romacs answers the selected frame, with coordinates in the primary window; X and Y are nil while the pointer is outside it, or with no display. `set-mouse-position` and `set-mouse-pixel-position` make GNU's checks and move nothing, as pgtk's (`pgtkterm.c:397` is empty). `crates/romacs-vm-gui/harness/server_live.py` (mouse) moves a real pointer against pinned GNU | **Package-observable** only for the pointer over a child frame | <!-- symbols: mouse-position, mouse-pixel-position, mouse-position-in-root-frame, set-mouse-position, set-mouse-pixel-position -->
