## Observable by a package: scoped GUI command bootstrap

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-GUI-SIMPLE-SCOPE | GUI GNU loadup | GNU's dumped image normally supplies all of simple.el. | When simple.el stops at an unrelated unsupported declaration, the GUI separately evaluates the exact source forms for the line-motion command dependency group. Existing definitions are never replaced; the original partial-file result and each supplemental source line remain reported. | Decouples C-a/C-e from generic-function bootstrap without native command stand-ins or edits to GNU Lisp; scoped source-loading and original command-loop tests cover it. |
