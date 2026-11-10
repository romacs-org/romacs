## Native leaf batch boundaries

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-LEAF-MAPCONCAT | `mapconcat` property preservation and absent bool-vector constructor | Concatenation preserves properties from mapped strings and separators; bool-vectors are valid sequences. | Inherited native concat drops properties (`mapconcat/0021`, GNU bold versus nil). Bool-vector witness stops at missing make-bool-vector (`/0016`). | Shared concatenation/property and representation boundaries; live corpus and exact witnesses in `evidence/leaf-sequences/divergences.json`. Pinned GNU itself aborts when a mapping callback truncates its active list; excluded crashing witness retained in pin-probes.json, not synthesized as a condition. | <!-- symbols: mapconcat, concat, make-bool-vector -->
