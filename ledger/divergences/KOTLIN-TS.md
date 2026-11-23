## Observable by a package

| # | mechanism | GNU | Romacs | why |
|---|---|---|---|---|
| KOTLIN-TS | Kotlin editing | No bundled Kotlin major mode or Kotlin file associations | `kotlin-ts-mode` derives from `prog-mode` and uses the bundled Kotlin grammar for indentation, highlighting, definition movement and Imenu. Visiting `.kt` or `.kts` selects the mode when the grammar is available. | Provide Kotlin editing on a fresh install. Packages can observe the mode, file associations and parser. | <!-- symbols: kotlin-ts-mode, kotlin-ts-mode-maybe -->

Syntax parsing treats interpolation as part of the string. Unescaped nested
quotes in `${...}` can affect syntax state until the syntax engine supports
interpolation.

Editing a raw string while narrowing hides its unpropertized opener can
leave later code classified as string content after widening. GNU's
`python-mode` shows the same behavior for triple-quoted strings. This probe
prints `t` under `-Q --batch` on pinned GNU revision
`45b0f7699df36b3849edc3bdef37d4cb9e75b9ea`. Calling
`(syntax-ppss (point-max))` before narrowing makes it print `nil`.

```elisp
(with-temp-buffer
  (insert "def f():\n    s = \"\"\"before\n    abc \" def\n    ghi\n    jkl\n    \"\"\"\n    following = 0\n")
  (let ((python-indent-guess-indent-offset nil)) (python-mode))
  (save-restriction
    (goto-char (point-min))
    (search-forward "abc")
    (let ((beg (match-beginning 0)))
      (search-forward "jkl")
      (narrow-to-region beg (match-beginning 0)))
    (goto-char (point-min))
    (insert "x")
    (syntax-ppss (point-max)))
  (goto-char (point-min))
  (search-forward "following")
  (prin1 (nth 3 (syntax-ppss (match-beginning 0)))))
```

Even when syntax is already propertized, an edit at the lower edge of the
restriction can leave stale string state after widening. Adding a fourth
quote just after a Kotlin raw-string closer needs to move the closing
fence. Inserting text just inside an escaped identifier that contains a
quote needs to revisit the opening backtick. In both cases the syntax
propertizer cannot reach the preceding text while narrowed.

In GNU's `python-mode`, a fourth quote after the closer starts an ordinary
string in both edited and fresh buffers. To reproduce the narrowing
limitation on the same GNU revision, complete a new triple-quoted opener
immediately after a closer by changing five consecutive quotes to six.
This probe prints `t`. With the `narrow-to-region` call omitted, or with
the final text inserted in a fresh buffer, it prints `nil`.

```elisp
(with-temp-buffer
  (insert "def f():\n    x = \"\"\"before\"\"\"\"\"body \" mid\n    \"\"\"\n    following = 0\n")
  (let ((python-indent-guess-indent-offset nil)) (python-mode))
  (syntax-ppss (point-max))
  (goto-char (point-min))
  (search-forward "before\"\"\"\"\"")
  (save-restriction
    (narrow-to-region (point) (point-max))
    (insert "\"")
    (syntax-ppss))
  (goto-char (point-min))
  (search-forward "following")
  (prin1 (nth 3 (syntax-ppss (match-beginning 0)))))
```

Making `syntax-propertize` work on the widened buffer is future work for
the shared syntax engine.
