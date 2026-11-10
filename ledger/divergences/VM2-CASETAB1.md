## VM2 standard case-table startup stage

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-CASETAB1 | Standard table initialization stage | `casetab.c:init_casetab_once` constructs four ASCII-only tables, subsequently enriched by GNU Lisp before the dumped oracle starts. The pinned dumped oracle maps character 192 to 224. | Lazily constructs and independently roots all four ASCII startup tables and the canonical-to-equivalence link. Before the GNU enrichment runs, `(aref (standard-case-table) 192)` is nil, not the dumped oracle's 224. | Preserve the actual pre-Lisp initialization contract rather than copying post-load Unicode data; table printer representation remains VM2-CT1. | <!-- symbols: standard-case-table -->
