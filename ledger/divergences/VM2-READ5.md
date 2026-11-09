## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-READ5 | `read-from-string` error payloads | GNU reading `(a . b c)` signals `(invalid-read-syntax "expected )")`. | VM2 signals `(invalid-read-syntax "dotted list requires one tail at byte 8")`; the same condition symbol does not imply matching condition data. `read-from-string/0170` is explicitly pinned `DATA`. Other reader errors may likewise expose internal messages/byte positions rather than GNU payloads. | **Package-observable staged diagnostic contract:** preserve complete actual fault data in evidence rather than hide payload differences as printer behavior. | <!-- symbols: read-from-string -->
