## Observable by a package: columns and minibuffer identity

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-CURRENT-COLUMN | `current-column`, `current-indentation`, `move-to-column`, `indent-to` | GNU accounts for display tables, invisibility and composition, and `current-column` caches its last answer by point and modification count (so a `tab-width` change alone can leave it stale). | Each character counts as GNU's CHARACTER_WIDTH counts it, by `char-width-table` (characters.el's) and `ctl-arrow`, with tab stops, without moving point: every code point's column matches pinned GNU (`tests/char_width_sweep.rs`). Display tables, invisibility, composition and display-property overrides are not projected, and non-UTF-8 byte text refuses. | Plain-text command-motion slice, not a complete display-table implementation. |
