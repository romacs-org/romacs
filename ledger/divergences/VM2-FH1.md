## Observable by a package

| ID | Area | GNU behavior | Romacs behavior |
| --- | --- | --- | --- |
| VM2-FH1 | Cyclic file-name-handler-alist | GNU fileio.c:370 walks cons tails until completion or user quit; a circular outer alist does not terminate by itself. | `find-file-name-handler` matches acyclic alists, operation metadata, inhibition and match-data preservation, but refuses a detected circular outer alist with `(error "VM2 cyclic file-name-handler-alist is unsupported")`. Live GNU observations and regression tests are in `crates/romacs-vm/evidence/file-name-handler/`. |
