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
