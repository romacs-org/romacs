## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-MEMCOUNT1 | `memory-use-counts` units | GNU `Fmemory_use_counts` (`src/alloc.c:7212`) counts its allocator's objects, intervals among them, and its string characters | VM2 counts its own allocations: conses, floats, strings and their bytes, and every other object or closure as vector cells; text properties are not intervals here, so INTERVALS stays 0; symbols are counted from the symbol table | Absolute values differ, as they do between GNU builds; the shape, monotonic growth and which counter moves match, except INTERVALS | Only deltas are meaningful; `window-tool-bar` reports them as debugging statistics | <!-- symbols: memory-use-counts -->
