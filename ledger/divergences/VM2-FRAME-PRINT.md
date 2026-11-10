## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-FRAME-PRINT | Opaque frame and terminal printing | GNU prints runtime frame names, addresses and terminal identifiers | VM2 prints `#<frame initial>` and `#<terminal initial>` for its actual initial objects; these are diagnostic labels, not GNU printer parity | Package-observable printed representation; identity and type queries agree on the supported initial graph. The oracle replay excludes process-specific printer rows but retains their raw GNU results | See `tools/primh/frame-evidence` under `crates/romacs-vm`; do not infer identity by comparing these labels | <!-- symbols: prin1, prin1-to-string, selected-frame, frame-terminal -->
