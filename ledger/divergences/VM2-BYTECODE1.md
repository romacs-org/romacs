## VM2 GNU bytecode representation

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-BYTECODE1 | GNU bytecode object predicate substrate | `src/data.c:531` accepts GNU closure objects with string bytecode; `#[0 "\\300\\207" [42] 1]` is one. | `byte-code-function-p` is callable but returns nil for VM2's representable objects; the positive reader witness signals `invalid-read-syntax`. VM2 register-code closures are not GNU bytecode. | GNU bytecode object representation, reader and producers are absent; this is a bounded negative-domain implementation, not bytecode execution support. | <!-- symbols: byte-code-function-p -->
