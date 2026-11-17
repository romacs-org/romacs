## Observable by a package: charset-era coding sequences

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-CODING-COMPOSITION | composition sequences in ISO-2022 and emacs-mule text | Decoding reads a composition sequence (ISO-2022's `ESC 0` .. `ESC 1` and their kin, emacs-mule's composition header) into the composed characters with a `composition` property: pinned GNU 32.0.50 decodes `"x\e0ab\e1y"` by `iso-2022-7bit` to `"xaby"`, `ab` composed | Decoding signals `VM2 ISO-2022 composition sequences are not implemented` (or the emacs-mule counterpart) at the sequence, from `crates/romacs-vm/src/prim/leaf/coding_mule.rs` | Composition sequences come from the charset-era codings that wrote compositions into files; UTF-8 text, which modern packages exchange, has none | <!-- symbols: decode-coding-string, decode-coding-region -->
