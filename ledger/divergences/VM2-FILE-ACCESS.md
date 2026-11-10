## Filesystem predicate boundary

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-FILE-ACCESS | `file-accessible-directory-p` path domain | Expands buffer-relative names, dispatches matching file handlers and handles native filename bytes. | Checks the actual kernel `PATH/./` accessibility (including directory/search permission and symlinks), but dispatches expansion and its own handler as GNU does; non-Unicode names are refused. `setq default-directory` works in this expander. | Reuses the existing filesystem subset rather than inventing successful answers for unsupported names/handlers; live pinned matrix and mutation proof are in `crates/romacs-vm/evidence/file-accessible-directory.md`. | <!-- symbols: file-accessible-directory-p -->
