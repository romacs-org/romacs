## Observable by a package: save batch (WIP, not accepted)

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-SAVE-PATH | Save-side path expansion, coding and handlers | GNU dispatches expansion/directory helpers and supports filename coding. | Expansion and its helpers (`directory-file-name`, `file-name-nondirectory` in expand_cp_target) dispatch their handlers in GNU's order, through Lisp where one is selected, then the operation's own handler runs. Non-UTF-8 filename coding and nonrepresentable byte paths remain refused. | The full filename codec is not implemented. `crates/romacs-vm/evidence/filesave/README.md`, `evidence/handler-local/`. |
