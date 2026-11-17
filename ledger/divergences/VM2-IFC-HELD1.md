## Buffer lifecycle remaining boundaries

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-IFC-HELD1 | a buffer's overlays while `insert-file-contents` decides its coding | GNU makes the empty buffer unibyte while `set-auto-coding-function` reads the file's bytes; its markers stay at BEG | Romacs's unibyte buffer takes the markers, at BEG (`insert_from_gap_1` moves none), and holds the empty multibyte buffer's overlays aside until the unwind restores it (`fileio_set_multibyte_flag`). While they are held, overlay queries on that buffer signal, and `set-buffer-multibyte t` signals `cannot decode buffer while insert-file-contents decides its coding`, where GNU succeeds. The buffer and its markers are intact afterwards (`crates/romacs-vm/tests/real_ert_load.rs`) | GNU's `set-auto-coding` only reads the buffer, so only a custom `set-auto-coding-function` can see this | <!-- symbols: insert-file-contents, set-auto-coding-function -->
