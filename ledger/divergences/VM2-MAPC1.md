## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-MAPC1 | `mapc` inherited callback and sequence substrate | GNU `fns.c:3569` maps bool-vectors and accepts the Lisp `identity` callback. | Cold VM2 has no `identity` until GNU subr.el loads. Live primh `mapc/0001`, `/0004`, `/0005`, `/0006`, `/0054` retain their `(void-function identity)` pins. Bool-vector mapping now preserves return identity and observes callback mutation; `/0080` is exact and its missing pin was removed. | Package-observable cold-bootstrap and representation gaps; do not replace GNU's Lisp `identity` natively. Evidence: `crates/romacs-vm/tools/primh/evidence/mapc.json`. | <!-- symbols: mapc, identity, make-bool-vector -->
