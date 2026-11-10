## Observable by a package: columns and minibuffer identity

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-MINIBUFFER-PRED | minibufferp | GNU retains every initialized minibuffer and recognizes active enclosing recursive levels. | The predicate recognizes native minibuffer-window buffer identities and current activation. Hidden buffers from enclosing recursive levels are not tracked by this predicate. | Uses the current native window model rather than buffer-name heuristics. |
