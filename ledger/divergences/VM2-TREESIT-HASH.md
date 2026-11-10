## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-TREESIT-HASH | `sxhash-equal` of tree-sitter nodes | `equal` compares nodes with `treesit_node_eq` (`fns.c:2979`) but `sxhash-equal` hashes the node object's address (`fns.c:5565`), so two `equal` nodes hash differently and an `equal` hash table misses one. | A node hashes by its tree and node identity, the pair `ts_node_eq` compares, so `equal` nodes hash alike and an `equal` table finds them. | A package keying an `equal` table by nodes finds what GNU would miss; nothing can rely on GNU's miss. | <!-- symbols: sxhash-equal, treesit-node-eq -->
