## VM2 explicit garbage collection

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-GC-ACCOUNTING | Native heap accounting | GNU counts its own typed allocation pools and vectorlike objects. | Counts describe Romacs storage: cons arena entries/reserve; symbol slots excluding private binding slots and freed slots; string headers and actual string bytes; ordinary vector arena entries and element slots/reserve; float arena entries; both buffer variants. Mixed object-arena reserve is not attributed to strings or floats (their typed free pools are empty). SIZE is the corresponding fixed Romacs storage, excluding variable backing allocations; buffers use the shared object-arena slot size. Non-vector Rust objects are not claimed as GNU pseudovectors. | Numeric results and pseudovector population differ; this is heap accounting, not a GNU heap-size emulator. No startup objects are allocated. | <!-- symbols: garbage-collect -->
