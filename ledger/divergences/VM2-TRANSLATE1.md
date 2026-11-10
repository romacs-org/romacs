## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-TRANSLATE1 | `translate-region-internal` outside Unicode text | `editfns.c:2509` translates unibyte buffers byte by byte and may put raw-byte characters (a unibyte table's non-ASCII bytes, BYTE8) into a multibyte buffer | VM2 translates multibyte buffers as GNU does (in-place one-for-one rewrites keep properties; other replacements follow `replace_range`'s point and marker rules); a unibyte buffer, or a result that is not a Unicode character, signals an explicit VM2 error before that replacement | A VM2 multibyte buffer holds Unicode text; ucs-normalize and other callers map characters to characters | <!-- symbols: translate-region-internal, translate-region -->
