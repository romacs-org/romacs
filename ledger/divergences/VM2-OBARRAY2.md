## Detail

| crate | file | entries |
|---|---|---|
| VM2-OBARRAY2 | Non-Unicode obarray names | GNU accepts arbitrary Emacs strings, comparing both exact bytes and character count. | Custom obarrays retain exact input-name objects and compare bytes/characters, but construction still requires a Unicode-decodable name for the existing symbol-slot metadata. `(intern (unibyte-string 255) (obarray-make))` signals `not-unicode-string`; GNU returns a symbol. The initial name table also conflates equal UTF-8 bytes with differing character counts. Live transcript: `evidence/obarray-substrate/divergence-probes.json`, `raw-name`. Existing name-based constant checks described in VM2-BOOT4 can also affect custom symbols. | **Package-observable symbol-substrate boundary**; no byte coercion or invented replacement symbol is returned. | <!-- symbols: intern, intern-soft, unintern, symbol-name -->
