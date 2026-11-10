## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-READGRAPH1 | Numbered cyclic literals | GNU resolves forward/self references during object construction. | Completed aliases retain identity; forward/self references explicitly signal undefined numbered reference. | No fake cyclic graph and no flattening into equal copies. | <!-- symbols: read, read-from-string -->
