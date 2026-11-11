## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-READPROP2 | Duplicate reader property keys | GNU preserves repeated keys in a string literal's text-property plist, including identical values and interleaved keys; property lookup returns the first occurrence. | The reader signals `error` with `Reader cannot retain duplicate text property keys` instead of silently collapsing repeated symbol keys. | `Properties` stores one layer per key name and cannot faithfully retain repeated plist entries; changing that shared representation is outside this reader repair. Recorded pinned GNU cases and a reader regression cover duplicates, mixed keys, identical values, and nil keys. | <!-- symbols: read, read-from-string, text-properties-at -->
