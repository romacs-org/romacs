## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-WIN2 | `delete-other-windows-internal` re-layout of an internal WINDOW | GNU resizes the subtree of an internal WINDOW that takes ROOT's place through Lisp `window--resize-root-window` and `window-resize-apply` (`src/window.c:3467`) | VM2 scales WINDOW's children proportionally with its frame-resize plan, in Rust | **Package-observable** only through the exact pixel split of the surviving children, which can differ by rounding from GNU's resize | The primitive must not evaluate Lisp | <!-- symbols: delete-other-windows-internal -->
