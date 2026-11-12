## World snapshots

| # | Mechanism | GNU | Romacs | Boundary |
|---|---|---|---|---|
| VM2-WORLD1 | `emacs-build-time` in a VM restored from a world | A dumped GNU reports the time its dump was made: `version.el:79-84` runs in loadup, and pdumper restores its value without evaluating it again. | A VM booted from source reports its own clock at `version.el`; one restored from a world reports the time the world was built, as a dumped GNU does. | **Package-observable** only through that one value: every other value a restored VM holds is a function of the binary, the recipe and its keyed host facts, or is rebuilt (`romacs-world-restart`). test/gnu/tools/world-diff-allow.txt lists it. | <!-- symbols: emacs-build-time -->
