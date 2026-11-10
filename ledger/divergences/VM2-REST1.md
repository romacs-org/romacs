## Detail

| crate | file | entries |
|---|---|---|
| VM2-REST1 | R160 opaque `&rest` argument tails | `src/eval.c:3421-3439` constructs a fresh `Flist` and binds it; `src/bytecode.c:542-549` pushes a fresh list for excess arguments, or nil for an empty tail | VM2 keeps an unobserved tail in a traced, reusable argument stack and materializes one ordinary list before a binding read, capture, store, mutation, return, or onward call; unknown uses conservatively materialize | **No list-semantic divergence:** a package retaining or mutating its rest binding sees an ordinary fresh list with shared identity for all aliases, including after compaction. Allocation/GC accounting can observe fewer conses for unused tails; no timing or GNU speed claim | <!-- symbols: lambda, funcall, apply -->
