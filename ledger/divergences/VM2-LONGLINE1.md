## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-LONGLINE1 | Long-line optimizations | xdisp.c narrows display and font-lock around a line longer than `long-line-threshold` and sets the buffer's flag that `long-line-optimizations-p` reads | Romacs's redisplay has none: `long-line-optimizations-p` is always nil, so `line-move`, font-lock and jit-lock take their ordinary paths on any line length | **User-observable** only as speed on very long lines; Lisp sees GNU's answer for a buffer without long lines | <!-- symbols: long-line-optimizations-p, long-line-threshold -->
