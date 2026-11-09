## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| — | Custom theme stack | enabled-theme/user priority stack, `custom-set-faces`, `theme-face` records | one immutable theme spec composed with one explicit `UserOverrides` value; user preferences survive theme replacement | R90 removes the mechanism and keeps the preferences; **no speed comparison is claimed** (the earlier 3.7 ns figure was retracted — it compared a prebuilt swap against GNU's full rebuild) |
