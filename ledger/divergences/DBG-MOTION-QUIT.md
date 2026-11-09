## GNU ERT batch execution

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| DBG-MOTION-QUIT | A long motion quits | GNU's display iterator polls nothing, so `vertical-motion` and its kin run to the end, and C-g waits | `vertical-motion`, `posn-at-point`, `pos-visible-in-window-p`, `line-pixel-height`, `window-text-pixel-size` and `buffer-text-pixel-size` poll every 1024 elements (`Vm::native_poll`), through the quit owner, so `inhibit-quit` holds a C-g and a debug event comes through as in a regexp search. To a package: one of these may signal `quit`. Redisplay does not poll for quit: it never unwinds half done, and the quit waits for the next safepoint | A session must not freeze in a motion nothing can interrupt | <!-- symbols: vertical-motion, posn-at-point, pos-visible-in-window-p, line-pixel-height, window-text-pixel-size, buffer-text-pixel-size -->
