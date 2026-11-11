## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-READSTREAM1 | Reader stream adapters | GNU `lread.c:257-516,2457-2563` reads strings, buffers, markers, callbacks and dynamic standard-input with observable get/unget. | Shared integer-code grammar now handles string bounds, live Unicode/byte buffers, live Unicode-buffer markers and reentrant callbacks with rooted suspended engines. Callbacks may collect, signal and throw. `read t` explicitly signals an unwired interactive-input error; markers in byte buffers explicitly signal; loader file decoding and mixed raw-byte text buffers remain separate boundaries. The host Rust-str compiler adapter retains its prior multibyte-ASCII literal policy, unlike GNU Lisp-stream read. | No snapshot callback substitute or nil success. Text buffers still require strict Unicode (`text.rs`); the string codec covers the GNU 22-bit domain. | <!-- symbols: read, read-from-string, standard-input -->
