## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-WS1 | `set-window-start` refuses a non-nil NOFORCE | GNU accepts NOFORCE and simply skips forcing the start position: `if (NILP (noforce)) w->force_start = true;` (`src/window.c:1967-1968`), so a non-nil NOFORCE means redisplay may move the start again rather than being pinned to it; the call otherwise succeeds and returns POS | VM2 signals `VM2 NOFORCE redisplay selection is unsupported` (`crates/romacs-vm/src/window.rs:844`), so the whole call fails instead of setting the start without pinning it | Yes. Any package calling `(set-window-start w pos t)` gets an error where GNU returns POS. `window-start` is left unchanged, and callers that ignore the return value see no scroll. Revertible by setting the start and omitting the force flag, which is the behaviour NOFORCE asks for. |
