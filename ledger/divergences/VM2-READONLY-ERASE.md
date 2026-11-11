## Observable by a package: owned local editor commands

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|---|
| VM2-READONLY-ERASE | `erase-buffer` rejection in a narrowed read-only buffer | The pinned GNU implementation widens before reporting `buffer-read-only`, changing the accessible region despite rejection. | The native buffer-level guard runs before widening, preserving text, point, restriction and modification ticks on rejection. Any non-nil `inhibit-read-only` bypasses the buffer-level guard; text-property protection is a separate boundary. | Deliberate failure-state preservation for editor-owned read-only views. Live pinned GNU probes distinguish the widening side effect; maintained text/byte-buffer tests assert the safer contract and typed condition/data identity. | <!-- symbols: erase-buffer, buffer-read-only, inhibit-read-only -->
