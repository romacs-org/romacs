# romacs-regex-engine: deliberate differences from GNU Emacs

Reference: GNU Emacs 32.0.50, `src/regex-emacs.c` and `src/search.c` (the
implementation, not the docstrings).

**One introduced by the linear-time matcher (R92 step 2), RX1 below.**
Everything else the matcher returns is GNU's: the linear matcher reproduces
the backtracker's leftmost-first priority and registers, including GNU's
empty-loop rule, and `tools/regex_oracle.py` checks it against live GNU
(106 cases, fresh process each, byte-compiled).

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

## Not observable

- The search loop.  A forward search whose candidates have cost more than
  `8 * span + 8192` (DFA bytes read, failure points popped, Pike positions)
  runs its remaining candidates in one unanchored pass of the linear matcher
  instead of one anchored attempt each.  Same match, same registers, same
  overflow (a candidate that could overflow is handed back to the
  per-candidate path).  Backward searches are unchanged.
- Captures in nullable loops (`\(a*\)*`) now run on the linear matcher,
  which models `CHECK_INFINITE_LOOP` exactly; they ran the raw backtracker.
- Master's Pike VM pruned GNU's empty loop iteration by pc, which reorders
  threads: `\(?:\|a\)*\(?:ab\)?` on `ab` gave (0 1) where GNU and the
  backtracker give (0 2).  It ran only after the backtracker's budget, so it
  was rare; the linear matcher gives GNU's (0 2).  A fix toward GNU, not a
  divergence.
- `C-g` is polled every 4,096 positions in the linear matcher (the
  backtracker polls at its jumps and fail sites, as GNU does).

## The extraction (step 1)

No behaviour changed in the extraction.  This crate is the engine that lived in
`romacs-core/src/emacs_core/text/regex/{emacs,dfa}.rs`, moved behind a trait
boundary without changing what it matches. Every difference from GNU that the
engine already had is unchanged and stays recorded where it was (the root
`DIVERGENCES.md` ledger and the engine's own comments).

What changed is only where host state is read, each read to the same value as
before in romacs-core:

| read | before (in core) | now | core's answer |  <!-- not-a-divergence -->
|---|---|---|---|
| quit poll | `eval::tls_quit_pending()` | `host::quit_pending()`, a per-thread probe | the same function, installed at runtime activation and by the test quit-flag installer |  <!-- not-a-divergence -->
| char-table tick for the DFA class memo | global `char_table_write_tick()` | `SyntaxLookup::table_write_tick` and `CaseTable::write_tick` | the same global tick, from both |  <!-- not-a-divergence -->
| case-canon translation | `chartable::translate_char` on a held `Value` | `CaseTable::translate` | the same call, inside core's `CharTableCase` |  <!-- not-a-divergence -->
| standard canon | core's `downcase_char_code_emacs_compat` | `emacs_downcase_compat` | core's function now delegates here: one definition |
| `EmacsChar`, `SyntaxClass`, the decoder helpers | core modules | this crate | core re-exports them unchanged |  <!-- not-a-divergence -->

A behaviour change made here later must add a row both here and in the root
`DIVERGENCES.md`, which is canonical.
