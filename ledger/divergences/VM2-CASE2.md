## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-CASE2 | `case-symbols-as-words` Boolean forwarding | GNU `casefiddle.c:syms_of_casefiddle` declares a Boolean-forwarded, automatically buffer-local variable, normalizing assigned non-nil values to t. | VM2 declares it special and automatically buffer-local, honoring local/dynamic truth values, but stores ordinary Lisp values: live `capitalize/0058` returns `(ok (t "Foo_bar"))` on GNU and `(ok (7 "Foo_bar"))` on VM2, pinned `VALUE`. | **Package-observable variable representation gap**, while the resulting capitalization matches. The inherited circular printer gap VM2-EV2 also pins `capitalize/0018`, `/0039`, `/0068` as `PRINTER`; these are not claimed as primitive parity. | <!-- symbols: capitalize case-symbols-as-words -->
