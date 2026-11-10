## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| RX1 | regexp search that GNU answers with "Stack overflow in regexp matcher" only after >1,066,664 failure-point pops | freezes (minutes: `a*a*a*b` over 100,000 `a`s), then the overflow error if the limit is ever reached | GNU's backtracker decides such a candidate under a 1,066,664-pop budget; past it, returns the linear matcher's answer (match or `nil`); overflows GNU reaches on its first descent are unchanged | R92/R93: "experienced by the end user as 'Emacs froze'"; reproducing that error means reproducing the freeze |  <!-- symbols: string-match, string-match-p, re-search-forward, re-search-backward, looking-at, looking-back -->

## RX1 — a parity run that would freeze GNU is abandoned

**GNU** (`regex-emacs.c`, `re_match_2_internal` with `ENSURE_FAIL_STACK`):
the backtracker searches depth first until it matches, exhausts every path,
or its fail stack passes `emacs_re_max_failures` (40,000 failure points),
when `re_search_2` returns -2 and `search.c` signals
`(error "Stack overflow in regexp matcher")`.  Whether the limit is reached
is only known by running the search to that point, which on `a*a*a*b` over
100,000 `a`s takes longer than anyone waits: the depth stays under the limit
while the backtracking is cubic.

**romacs**: where a candidate's paths run far enough that GNU's stack could
overflow (`fail_stack_may_overflow`), GNU's own backtracker still decides the
candidate, so the error survives -- but under a budget of
`4 * FAIL_STACK_ENTRY_LIMIT` = 1,066,664 failure-point pops.  GNU overflows
on its first deep descent, well inside that (every overflow case in
`fail_stack_parity_test` and the oracle still overflows under the budget);
a run that pops the whole budget without overflowing is a freeze.  romacs then
returns the linear matcher's answer for that candidate and finishes the
search linearly without the fail-stack model.

**Observable?** Only by a search that GNU would answer with the overflow error
*after* more than 1,066,664 pops -- minutes of "Emacs froze" first.  romacs
answers it (match or `nil`) instead.  Every overflow GNU reaches on its first
descent is unchanged and tested (`fail_stack_parity_test`,
`linear_test::a_linear_search_keeps_gnu_fail_stack_overflow`,
`linear_test::a_parity_run_that_would_freeze_gnu_is_abandoned`).

Live GNU 32.0.50 (b28750b822dc), `(string-match "a*a*a*b" (make-string N ?a))`:

| N | GNU | romacs (this crate, `regex_probe`) |  <!-- not-a-divergence -->
|---|---|---|
| 50,000 | killed at 20 s, no answer | `nil` |  <!-- not-a-divergence -->
| 100,000 | killed at 20 s, no answer | `nil` |  <!-- not-a-divergence -->
| 160,000 | killed at 20 s, no answer | `nil` |  <!-- not-a-divergence -->
| 400,000 | `(error "Stack overflow in regexp matcher")` in 0.013 s | `overflow`, same |  <!-- not-a-divergence -->

**Why**: R92/R93 -- the maintainer's own diagnosis is that this path is
"experienced by the end user as 'Emacs froze'"; reproducing an error that
GNU only reaches after the freeze would mean reproducing the freeze.
