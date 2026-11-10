## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-FRAME-MINI | Saved minibuffer selection is unsupported | GNU `minibuffer-selected-window` (`src/window.c:7153`) returns the live saved selection only during an active, selected minibuffer; `src/minibuf.c:827` updates it on real recursive entry | Unregistered: direct calls and funcall signal `(void-function minibuffer-selected-window)` rather than inventing nil from a depth counter | Package-observable refusal, not matching GNU's successful result. The overlay has a lexical calling-window but no native recursive-entry/unwind selection ownership | Preserve this refusal until the actual minibuffer lifecycle publishes and restores a traced selection; live GNU/VM evidence in `crates/romacs-vm/evidence/frame-refusals.json` | <!-- symbols: minibuffer-selected-window -->
