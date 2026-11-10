## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-BV1 | Bool-vector remaining representation boundaries | GNU reads and prints `#&` literals and serializes bool-vectors through its ordinary object machinery. | VM2 constructs packed bool-vectors and implements indexed access, predicates, counts, set operations, equality/hash, GC, and the tested sequence consumers. The `#&` reader branch, executable-image/protocol input encoding, and complete print-option fidelity remain absent; huge-allocation error text retains the inherited memory-full divergence. | Explicit partial substrate. Live pinned cases and per-primitive behavioral checks are in `crates/romacs-vm/evidence/bool-vector/`; no reader or keymap implementation is claimed. | <!-- symbols: bool-vector, make-bool-vector, bool-vector-p -->
