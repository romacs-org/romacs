## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-CT1 | Retained character-table structure | GNU retains 64/16/32/128 nodes and exposes raw ASCII/subtable slots and live traversal. | Reader and printer preserve raw `#^`/`#^^` nodes, completed numbered aliases, extras and explicit ASCII slots; standalone subtable objects are distinct. Nodes are traced and forwarded and existing interval writes remain compressed. Mapping refuses independent ASCII layouts and invalid imported geometry rather than risking a panic or reporting different contents. Image serialization of native table graphs remains explicitly rejected. | Unicode-property lazy block decoding, complete graph cycles and structural equality/hash remain unimplemented. Inverted range writes retain the established no-op divergence. | <!-- symbols: make-char-table, set-char-table-range, char-table-range, map-char-table, map-keymap, copy-sequence, aref, aset -->
