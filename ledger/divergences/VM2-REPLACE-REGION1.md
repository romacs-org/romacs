## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-REPLACE-REGION1 | `replace-region-contents` past MAX-SECS with a vector SOURCE | `editfns.c:2155` falls back to replacing the region with SBUF's whole accessible text, not the SBEG..SEND the vector names | VM2 falls back to SBEG..SEND; everything else (GNU's comparison and edit script, markers, point, undo, hooks, MODIFF, and INHERIT reaching replace_range as its adjust-match-data flag) follows the pinned source and is compared with it | The fallback only runs when the comparison times out or lacks memory, and replacing with text the caller did not name would corrupt the region | <!-- symbols: replace-region-contents -->
