## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| SMTP-QUIT1 | SMTP connection cleanup | `smtpmail-via-smtp` sends QUIT during cleanup and waits for a reply with no time limit | The cleanup waits at most `smtpmail-quit-timeout` seconds (default 2) for the reply, then deletes the process. Zero skips the wait. A second C-g interrupts the wait and preserves any earlier failure or quit | A silent peer cannot hold up cancellation or the return from a successful send | <!-- symbols: smtpmail-via-smtp, smtpmail-read-response, smtpmail-quit-timeout -->
