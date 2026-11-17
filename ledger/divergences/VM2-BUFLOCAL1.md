## VM2 buffer-local substrate subset

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-BUFLOCAL1 | Native local-variable inventory | Enumerates every native buffer slot in native order. | Every per-buffer slot plus ordinary locals, in symbol-slot order; details below. | Romacs keeps locals by symbol, not in a native buffer structure. | <!-- symbols: buffer-local-variables, buffer-local-value, local-variable-p -->

GNU
`src/buffer.c:1485` enumerates ordinary locals plus every native buffer slot,
in buffer-alist/native-slot order. VM2 enumerates ordinary locals and every
`DEFVAR_PER_BUFFER` slot local in the buffer, in symbol-slot order; a
package can observe the ordering. `buffer-local-value`/`local-variable-p` for unrepresented native
buffer fields retain their existing ordinary-variable behavior. Why: this
lane adds clear-one storage and modeled slots, not the entire GNU buffer
structure. The live-GNU corpus compares explicit supported projections,
never equates that corpus with complete inventory parity.
