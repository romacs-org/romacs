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
