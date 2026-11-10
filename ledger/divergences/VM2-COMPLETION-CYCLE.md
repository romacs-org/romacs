## Observable by a package: circular completion tables

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-COMPLETION-CYCLE | Circular list completion tables | GNU traverses until an exact test match, a decisive common prefix, or user quit; some circular inputs never finish. | Decisive early results are preserved. Traversal that would revisit a detected cycle signals `circular-list`. | Bounded native traversal with callback state traced across GC. `tests/completion.rs` covers both early-result cases. | <!-- symbols: try-completion, all-completions, test-completion -->
