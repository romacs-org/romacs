## VM2-SQLITE1 — Bundled SQLite library

sqlite.c's primitives run on SQLite 3.53.2, compiled from the amalgamation
bundled with `libsqlite3-sys` 0.38, so no system library is needed. GNU pin
`45b0f7699df36b3849edc3bdef37d4cb9e75b9ea` links the host's library (3.54.0
on the reference host). `sqlite-version` and SQLite's own error messages
follow the bundled version. Text columns are decoded by the native `utf-8`
decoder with line-end detection, as GNU's `utf-8` coding system decodes
them, without looking `utf-8` up: a Lisp redefinition of that coding system
does not affect rows.
