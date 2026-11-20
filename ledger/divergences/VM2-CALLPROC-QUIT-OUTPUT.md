## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-CALLPROC-QUIT-OUTPUT | The output a quit in `call-process` leaves | callproc.c `call_process` fills a buffer of 16 to 64 KiB with `emacs_read_quit` before it inserts it; a C-g during that fill unwinds from the read, so the bytes of the partly filled buffer are dropped and only the blocks inserted before it stay in the buffer | `call-process` keeps every byte it read before the quit: the wait ends with the quit beside the bytes (`prim/leaf/process.rs` `call_process_wait`), `romacs--call-process-run` returns `(quit . BYTES)`, and `call-process` inserts them, decoded as any output, before it signals the quit (`lisp/vm/subprocess.el`) | Keeping all a command wrote before C-g loses nothing GNU would have kept, and a package sees at most more of the child's output, never less | <!-- symbols: call-process, call-process-region -->
