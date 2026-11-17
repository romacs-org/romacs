## Buffer lifecycle remaining boundaries

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-INDIRECT1 | indirect buffers share only multibyte text | GNU makes an indirect buffer of any buffer and lets `set-buffer-multibyte` convert a base buffer's text for all its indirect buffers | Romacs shares a multibyte (UTF-8) buffer's text only: `make-indirect-buffer` of a unibyte buffer signals `VM2 has no indirect buffer of a unibyte buffer`, and making a base buffer unibyte while it has indirect buffers (by `set-buffer-multibyte` or `insert-file-contents`'s coding toggle) signals `VM2 cannot make a buffer sharing its text unibyte`, before anything changes. In an indirect buffer `set-buffer-multibyte` signals GNU's own error | No caller is known to make an indirect buffer of unibyte text; comint, org and markdown-ts use multibyte ones (`crates/romacs-vm-gui/tests/indirect_buffer.rs`) | <!-- symbols: make-indirect-buffer, set-buffer-multibyte -->
