## Ordered ERT measurement

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-RECORD-SCOPE | Existing unavailable generic predicates | GNU arrayp and sequencep both return nil on records; read reads their printed spelling. | Native arrayp now returns nil on records and make-list supports the record boundary probes. Bare VM2 still has no sequencep or read primitive; the unchanged record corpus honestly retains six MISSING results for those two symbols. | These existing missing APIs are not represented as passing record parity. | <!-- symbols: recordp -->
