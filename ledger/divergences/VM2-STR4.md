## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR4 | String property inspection across concatenation | GNU copies string interval properties into concat output. | Closed for supported string encodings: ASCII fast-path and Unicode/mixed sequence concatenation now retain property ranges and values, including GC retention. The separate VM2-STR2 mixed raw-byte/Unicode encoding refusal remains. | Maintained `crates/romacs-vm/tests/concat_properties.rs` and Dired semantic-face regressions cover offsets, source preservation and collected property values. | <!-- symbols: concat, propertize, put-text-property, get-text-property, text-properties-at -->
