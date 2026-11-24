## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-WIN2 | Prepared subtree extents in `delete-other-windows-internal` | GNU prepares the subtree through `window--resize-root-window` (`src/window.c:3595`). If fixed-size children prevent a resize, the old prepared sizes can still pass the tiling check at the old extent. GNU then expands the root without filling it with its children. | Romacs also requires the prepared extent to match the target before applying it. It retries with `IGNORE=t` if needed. A bare VM uses the Rust geometry planner. | **Package-observable** through the pixel edges of the surviving children. `romacs-delete-other-windows-expands-fixed-height-subtree` seeds the old prepared sizes and checks that the children fill the expanded root. | The children must tile the replacement area. Bootstrap needs geometry before Lisp sizing policy is loaded. | <!-- symbols: delete-other-windows-internal -->
