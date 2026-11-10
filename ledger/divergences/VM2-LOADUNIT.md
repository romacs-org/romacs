## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-LOADUNIT | Constant identity across top-level forms | GNU's `.elc` shares an `equal` quoted constant between the forms of one pending output unit; a source load never does. | Equal quoted constants of different top-level forms are never `eq`; within one function they are coalesced, as bytecomp's constant vector is. | Code that tests `eq` between literals of two top-level forms; none is known. | <!-- symbols: eq -->
