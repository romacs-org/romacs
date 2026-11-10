## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-BUFTEMP1 | Nested hidden temporary buffer names | GNU `buffer.c:1289-1298` randomizes an occupied space-prefixed name; live `buffer-name/0034` records a random numeric suffix such as `" *temp*-862148"`, while first-level `/0033` records `" *temp*"`. | Uniquifies deterministically as `" *temp*<2>"`; `/0034` pinned `VALUE`. Both levels are killed unwind-safely. | **Package-observable naming difference:** do not emulate randomness or manufacture oracle answers. | <!-- symbols: buffer-name -->
