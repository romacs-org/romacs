## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-NANPRINT1 | NaN payload formatting outside GNU print entry points | GNU prints payload-bearing NaNs, e.g. `1.0e+NaN`. | `string-to-number` retains GNU's NaN bits; `prin1`, `princ`, `print` and `prin1-to-string` now preserve the payload spelling. The general diagnostic display/formatter still uses the earlier canonical NaN spelling. All 626 numeric-parser GNU rows were re-recorded unchanged; the obsolete `string-to-number/0516` printer pin was removed, leaving its 31 inherited circular-printer pins. | Remaining diagnostic/formatter serialization gap, not a numeric parser or GNU print entry-point failure. | <!-- symbols: format, format-message, string-to-number -->
