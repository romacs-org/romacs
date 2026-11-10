## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-BUFAUTO1 | Autosaved modification state | GNU `buffer.c:1528-1533` returns `autosaved` when autosave and modification ticks agree; live `buffer-modified-p/0040` records `(ok autosaved)` after batch `do-auto-save`. | As GNU: `do-auto-save`, `set-buffer-auto-saved` and `restore-buffer-modified-p` with `autosaved` set the autosave tick, `recent-auto-save-p` compares it, and the auto-save failure time is per-buffer state (VM2-AUTOSAVE1). | `tests/real_ert_load.rs` `fileio_defuns_match_gnu`. | <!-- symbols: buffer-modified-p -->
