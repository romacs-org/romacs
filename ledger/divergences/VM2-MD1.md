## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-MD1 | cyclic `match-data` REUSE | unbounded CDR walk (compiled GNU exceeded a two-second external deadline) | signals `(error "Circular match-data reuse list")` | bounded malformed-input handling; **package-observable**, detailed source and live-GNU probe in the VM2 ledger |  <!-- symbols: match-data -->

## VM2-MD1 — Cyclic match-data REUSE is rejected

GNU `src/search.c:Fmatch_data` walks REUSE through `XCDR` without a cycle
check. In a fresh isolated GNU 32.0.50 byte-compiled lambda, the following
exceeded an external two-second deadline; the subprocess was killed and
reaped. The C loop has no progress condition for this circular tail.

```elisp
(progn (string-match "a" "a")
       (let ((r (list 0))) (setcdr r r) (match-data nil r)))
```

VM2 instead signals `(error "Circular match-data reuse list")`.
**Observable by a package: yes.** This keeps malformed optional reuse data
from wedging the agent entry point; it does not change finite proper-list
reuse, extension, or marker reseating. The bounded result has its own test
and a mutant that silently accepts the cycle. This is deadline/safety evidence,
not a speed comparison.
