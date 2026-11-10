## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| — | face remapping cycles | merge-point fallback/truncation | accepts transitive NORMAL/REMAP inheritance backrefs as GNU does (`xfaces.c:1976-2000`); rejects cycles *between distinct remapping entries*, returning the original paintable theme plus a diagnostic | keep valid solaire/mode-line remaps without silently accepting genuine entry cycles |  <!-- symbols: face-remapping-alist, face-attribute -->
