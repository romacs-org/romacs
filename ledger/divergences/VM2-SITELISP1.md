## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-SITELISP1 | Site Lisp directories | GNU adds its configure-time `PATH_SITELOADSEARCH` (`<datadir>/emacs/VERSION/site-lisp` and `<datadir>/emacs/site-lisp`, `src/epaths.in`) to `load-path` unless `--no-site-lisp`/`-Q`; only an Emacs installed under prefix `/usr` sees the distribution's `/usr/share/emacs/site-lisp` (the pinned GNU under its own prefix does not), and GNU loads a package's `.elc` from there | Romacs's runtime has no site-lisp of its own and adds none, as a GNU under another prefix; an init that wants a distribution package (notmuch from the system package) adds `/usr/share/emacs/site-lisp` itself, and Romacs then loads its `.el` sources (VM2-EV5), not the `.elc` another Emacs compiled | A system package's Lisp is one `add-to-list 'load-path` away; its `site-start.el`/`subdirs.el` are not run | <!-- symbols: load-path -->
