## Observable by a package: save batch (WIP, not accepted)

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-SAVE-RENAME | Rename confirmation and cross-device fallback | GNU may prompt on integer overwrite arguments and implements cross-device copying. | Collision with integer confirmation and EXDEV explicitly refuse without performing copy/delete. Linux atomic no-replace syscall failures propagate; no racy fallback. | Local atomic rename subset. `crates/romacs-vm/evidence/filesave/gnu.json`. |
