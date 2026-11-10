## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-ROUND1 | `round` and `ceiling` circular-input printer pins | GNU prints the circular domain value as `(1 2 1 2 . #2)` in the corpus rows | The rounding results match; VM2 cannot print that input, the printer limit of VM2-ELT3, so 72 `PRINTER` pins remain in each of `tools/primh/evidence/round.json` and `ceiling.json` under `crates/romacs-vm` | Printed form of a circular list only, not rounding | `PRINTER` pins are instrument limits, not verified parity | <!-- symbols: round, ceiling -->
