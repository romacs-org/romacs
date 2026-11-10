## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| — | circular face structures and input budgets | **SIGSEGV** at ~32,931 `merge_face_ref` frames; circular plist tail hangs instead; no native payload cap | typed `BoundedDatum` boundary rejects cycles, depth >32, >2^20 conses and >16 MiB expanded strings/symbols; printed source capped too | bounded native work instead of a crash or OOM |
