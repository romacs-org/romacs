## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-BUFDEAD1 | Selecting a deleted temporary buffer | GNU `buffer.c:2445` signals `(error "Selecting deleted buffer")`, recorded live in `buffer-name/0038` and `buffer-modified-p/0039`. | Rejects the deleted buffer but the existing legacy error adapter drops message data, returning `(error)`; both rows pinned `DATA`. | **Package-observable condition-data gap:** preserve rejection without broadening the legacy condition transport in this slice. | <!-- symbols: buffer-name buffer-modified-p -->
