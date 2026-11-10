## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-EV3 | `equal` supported object kinds | `fns.c:internal_equal` includes structural marker/overlay, record, bool-vector and char-table rules | Deep cons/vector/string/float comparison, cyclic graphs, string properties ignored; other runtime objects use identity. Two separate markers at the same position compare nil (measured in s124-vm-probes.json) | VM2 has no record/bool-vector/general char-table representation; marker/overlay/closure equality remains staged | <!-- symbols: equal -->
