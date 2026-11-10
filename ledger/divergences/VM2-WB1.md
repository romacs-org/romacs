## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-WB1 | `set-window-buffer` starts a reassigned window at position 1 | GNU restores the buffer's remembered `b->last_window_start` (`src/window.c:4376-4378`, `set_marker_restricted (w->start, make_fixnum (b->last_window_start), buffer)`), so redisplaying a buffer a window showed before resumes near its former top | VM2 keeps no per-buffer last window start, so `window-start` becomes 1 and the window scrolls to the top. `window-point` does follow GNU and takes the buffer's own point. | Yes — a package reading `window-start` after `set-window-buffer`, or relying on a restored scroll position, sees the difference; revertible by recording a per-buffer last window start |
