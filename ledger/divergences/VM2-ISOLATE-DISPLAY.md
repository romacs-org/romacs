## Observable by a package: isolates and frozen values

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-ISOLATE-DISPLAY | Frame and window queries, and standard output, in a worker | n/a. | A worker has the batch session's initial frame and terminal: `display-graphic-p` is nil, `frame-width` and friends answer as in `emacs --batch`. Its standard output is null (`host::spawn_isolate`): `ding`'s batch BEL and `princ` to `t` go nowhere, never into the spawner's output (a server's journal); `message` still reaches its stderr | A worker owns no display and no output of the session's. | <!-- symbols: display-graphic-p, ding -->
