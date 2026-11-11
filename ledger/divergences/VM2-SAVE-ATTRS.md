## Observable by a package: save batch (WIP, not accepted)

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-SAVE-ATTRS | File metadata and symlinks | GNU supports full filename/ID coding and attribute platform semantics. | Numeric and named-ID file-attributes and local symlink reads are implemented; unresolved user/group lookups fall back to numeric IDs. Non-UTF-8 symlink targets refuse; stat error and concurrent-change domains are not fully qualified. | Scoped native blockers; details in `crates/romacs-vm/evidence/savejourney/native-followup.md`. |
