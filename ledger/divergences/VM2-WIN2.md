## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-WIN2 | `delete-other-windows-internal` re-layout in a bare VM | GNU prepares the subtree through `window--resize-root-window` (`src/window.c:3467`) | Romacs also uses that preparation when window.el is loaded. A bare VM uses the Rust geometry planner. | **Bare-VM observable** through the pixel split of the surviving children | Bootstrap needs geometry before Lisp sizing policy is loaded. | <!-- symbols: delete-other-windows-internal -->
