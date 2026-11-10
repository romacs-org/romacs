## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| — | window-filtered face refs | considers window parameter context | a `:filtered` ref in a buffer's `face-remapping-alist` applies where the displaying window's parameter matches (VM2-FACE-REMAP); one in a `face` property or `face-attribute` query never matches, there being no window in that `View`/cache key | do not pretend a window-free cache is window-sensitive |  <!-- symbols: face-remapping-alist, face-attribute -->
