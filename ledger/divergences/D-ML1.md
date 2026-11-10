## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| D-ML1 | VM2 redisplay-time `set-window-vscroll` | `src/window.c:Fset_window_vscroll` changes the live window immediately; `src/xdisp.c:display_mode_element` calls `dsafe_eval`. GNU 32.0.50 PGTK direct `format-mode-line`: `inside=7 after=7` (`s34-gnu-mid-build.txt`); real redisplay: `inside=7 after=0` (`s50-gnu-redisplay.txt`) | VM2 phase-one `:eval` reads committed vscroll (`inside=0`), queues the pixel setter with one diagnostic naming `:eval` and `set-window-vscroll`, then explicit post-paint journal application commits 7 for frame N+1 (`after=7`). Last write wins, including writing the original value. Direct `format-mode-line` is not yet implemented | **Package-observable and deliberate:** two-phase immutable layout cannot accept a mid-build window mutation; this is the S34/GUI-FLIP ruling, not a GNU-parity claim | <!-- symbols: set-window-vscroll, window-vscroll, mode-line-format -->
