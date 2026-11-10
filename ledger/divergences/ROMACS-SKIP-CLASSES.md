## Observable by a package — Character-set motion

| # | Mechanism | GNU | Romacs |
|---|---|---|---|
| ROMACS-SKIP-CLASSES | `skip-chars-forward` / `skip-chars-backward` named classes | Accepts `[:class:]` sets | Signals an explicit error before moving point; literal/range/escape/complement sets are supported |

`skip-chars-forward` and `skip-chars-backward` implement literal characters,
ranges, escapes and complement sets, including Unicode, byte-buffer octets,
marker limits and narrowing. GNU's `[:class:]` syntax is not implemented;
these sets signal an error before moving point instead of approximating
syntax-table-dependent classification. `tests/skip_chars.rs` preserves that
explicit refusal. This subset supports GNU's unchanged `mail-header-extract`;
it does not establish full GNU mail-library or MIME-encoding compatibility.
