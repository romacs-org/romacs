## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| — | display-condition target | frame/backend conditions including tty, greyscale, monochrome and actual GTK feature | fixed truecolour graphical Wayland; `romacs`, `graphic`, `pgtk`, `gtk` match; `supports` always true; no tty/greyscale/mono | one render target. **Observable and deliberate:** selects GUI attributes for `custom-button` and 8 other specs that today's romacs leaves blank, because it reports window-system `romacs` while `faces.el` matches only `graphic` |
