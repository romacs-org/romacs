## VM2 buffer-local substrate subset

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-BUFLOCAL1 | Native local-variable inventory | Enumerates every native buffer slot in native order. | Every per-buffer slot plus ordinary locals, in symbol-slot order; details below. | Romacs keeps locals by symbol, not in a native buffer structure. | <!-- symbols: buffer-local-variables, buffer-local-value, local-variable-p -->
