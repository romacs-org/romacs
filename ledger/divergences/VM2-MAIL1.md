## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-MAIL1 | Mail | GNU ships message.el, smtpmail and Gnus; notmuch's Emacs package comes with notmuch | Romacs ships no mail interface of its own (`romacs-notmuch.el` and `romacs-mail.el` are gone): the real notmuch package, loaded from the user's init, runs on GNU's message.el and smtpmail with native TLS (VM2-TLS1) and HTML parts through shr (VM2-LIBXML1); `crates/romacs-vm-gui/harness/notmuch_parity_live.py` drives it side by side with pinned GNU on a synthetic store | A workspace launch (`--no-gnu-lisp`) has no mail commands | <!-- symbols: notmuch, notmuch-hello -->
