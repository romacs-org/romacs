## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-BACKQUOTE | Expanding a backquote template | backquote.el's `backquote-process` walks a template in Lisp each time the macro expands, and `eval` expands the templates of its form every time. | `backquote-process` is a primitive over the walk in `expand.rs`, a port whose expansions are `equal` to GNU's, and the compiler takes the walk's answer without calling the macro or taking the form through Lisp. It does so only while `backquote` is the macro the bootstrap left and `backquote-process` the primitive, so a redefinition or advice is called as in GNU. `backquote-listify` and `backquote-delay-process` stay GNU's. | A theme defined by `eval` of a table of backquoted faces, as Modus is, spent half of `load-theme` here; every load of source pays the same walk. |
