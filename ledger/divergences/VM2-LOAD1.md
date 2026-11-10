## Detail

| crate | file | entries |
|---|---|---|
| VM2-LOAD1 | Loader failures outside Lisp runtime execution remain textual | GNU runtime signals escaping `load`, `require` and `autoload-do-load` retain their condition and data | Lisp runtime loading now carries the original condition, data object and selected handler across loader reentry before cleanup. Live GNU comparisons cover ordinary errors, primitive type errors, cyclic aliases, require and autoload signals; returning-handler tests also retain the same error object through GC and unwind. The Rust embedder `load_file` interface still returns textual errors, and source-read/compile failures without a runtime escape use that interface. Compile-time macro-autoload placement remains VM2-LAZY4 | **Package-observable** for source-read/compile failures; ordinary runtime signals retain their payload | <!-- symbols: load, require, autoload-do-load, condition-case, signal -->
