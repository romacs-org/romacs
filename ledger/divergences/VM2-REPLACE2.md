## VM2 replace-match

| # | Mechanism | GNU | VM2 | Why |
|---|---|---|---|---|
| VM2-REPLACE2 | `replace-match` mixed encodings | GNU concat converts unibyte non-ASCII pieces into a multibyte result when required. | Mixed-encoding replacement explicitly refuses copied unibyte non-ASCII pieces when the result is multibyte. | Avoid silently corrupting raw-byte characters or text-property coordinates until conversion is implemented. |
