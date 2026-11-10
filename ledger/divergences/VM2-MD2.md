## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-MD2 | binary-buffer `match-data` default representation | markers, as for Unicode buffers | integers for binary buffers; Unicode buffers retain default markers; explicit integer mode appends either buffer as GNU does | unibyte buffers hold markers (VM2-EXCUR1, withdrawn), but their match data is not yet kept in them; **package-observable**, live-GNU transcript and chosen representation in the VM2 ledger |  <!-- symbols: match-data -->

## VM2-MD2 — Binary-buffer match-data uses integer positions

GNU `Fmatch_data` returns markers by default after both multibyte and unibyte
buffer searches. VM2 returns markers for Unicode text buffers, but integer
positions for binary byte buffers even when INTEGERS is nil. Its existing
marker/anchor substrate belongs to Unicode text buffers; pretending binary
positions were live edit-tracking markers would be false. With INTEGERS
non-nil, both buffer kinds return integer positions and append their buffer,
as GNU does. Search registers, `match-beginning`, `match-end`, and
`match-string` remain usable for binary buffers.

**Observable by a package: yes.** Fresh isolated byte-compiled GNU 32.0.50
versus compiled VM2, executable revision
`b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b`:

```elisp
(with-temp-buffer
  (set-buffer-multibyte nil) (insert "ab") (goto-char 1)
  (looking-at "ab") (integerp (car (match-data))))
;; GNU => nil
;; VM2 => t
```

The integer choice is deliberate and bounded to the binary storage edge.
It has a compiled red equality assertion and a reversal mutant, rather than
silently failing through the text-only marker adapter.
