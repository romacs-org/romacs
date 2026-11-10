## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-FR5-5 | `secure-hash-algorithms` lists algorithms VM2 cannot compute | `src/fns.c:6092` returns `(md5 sha1 sha224 sha256 sha384 sha512 sha3-224 sha3-256 sha3-384 sha3-512)`, every one supported by `secure-hash` | VM2 returns the same fresh list (corpus 9/9 exact), but `secure-hash` itself is not a VM2 function, so `files.el:7800` (`(memq uniq (secure-hash-algorithms))` then `(secure-hash uniq filename)`) signals `void-function secure-hash` | **Package-observable** when a caller acts on the list; the list is GNU's so that membership tests agree, and the gap is the missing `secure-hash` | <!-- symbols: secure-hash-algorithms, secure-hash -->
