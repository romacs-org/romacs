# format-message: bounded GNU compatibility slice

The production implementation is in `src/prims/format_message.rs`,
`src/prim/leaf/printing.rs`, and the shared formatter in `src/productivity.rs`.
`tests/format_message.rs` replays the pinned GNU evidence with a fresh VM and
collection floor 1 for each case. Recording the oracle alone is not a VM parity result.

## Reference and recording

The reference is GNU revision `45b0f7699df36b3849edc3bdef37d4cb9e75b9ea`:
`src/editfns.c` (`format-message` and its formatter), `src/doc.c`
(`default_to_grave_quoting_style` and `Ftext_quoting_style`), and `src/disptab.h`
(`DISP_TABLE_P`). Record from the repository root with the pinned wrapper:

```sh
scripts/build-gnu-oracle.sh
export ROMACS_GNU_ORACLE="${GNU_ORACLE_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/gnu-emacs-pin}/45b0f7699df36b3849edc3bdef37d4cb9e75b9ea/bin/emacs"
python3 crates/romacs-vm/tools/primh/record.py primh \
  crates/romacs-vm/tools/primh/format-message-specs/format-message.json
```

The wrapper verifies the binary's revision and its clean source checkout;
primh runs GNU with a private HOME. `../evidence/format-message.json` contains
64 live-recorded cases: 63 probes and the generated zero-argument arity case.
No expected answers are hand-written. The Rust replay checks the spec, generator,
driver, domains, replay-template, binary, and revision identities.

The two `format-message-probes-only` argument tags deliberately select no
cartesian-domain values. This is a bounded corpus, not universal format parity.
Character-list probes use `(append s nil)`: `string-to-list` is GNU Lisp code,
not a native primitive, and is not installed in this bare-VM test environment.

## Covered behavior

- Curve replaces every literal template backtick/apostrophe with U+2018/U+2019.
  Straight replaces template backticks with apostrophes; grave changes neither.
  Inserted `%s`/`%S` text is not requoted, and ordinary `format` is unaffected.
- Non-nil explicit styles bypass the capability flag and display table. Unknown
  symbols and nonsymbol values select curve; style symbols use identity, not
  their printed names.
- Nil style consults `internal--text-quoting-flag`. A false flag selects grave.
  With a true flag, a valid standard display table mapping U+2018 to the
  one-element integer vector `[96]` selects grave; other mappings select curve.
  Valid tables have the interned `display-table` purpose and exactly 18 extra
  slots. Probes cover defaults, inherited entries, wrong purposes, malformed
  tables/entries, and lexical evaluation with dynamically special bindings.
- `%s`, `%S`, `%d`, `%%`, nested printed data, ignored surplus arguments, empty
  templates, initial type/arity errors, dynamic restoration, and primitive
  independence from a rebound Lisp `format` function are covered.
- ASCII unibyte output remains unibyte where GNU does; curved output is
  multibyte. Combining raw high octets with curved quotes promotes each octet
  to its GNU byte8 character, without UTF-8 decoding. For example, octet 200
  between curved quotes yields character codes `(8216 4194248 8217)`.
  The corpus includes raw `[128, 195, 169, 255]`, valid-UTF-8 octets `[195, 169]`,
  raw bytes in the template, and ordinary-format controls.
- Startup declarations use the existing special-variable mechanism and allocate
  no Lisp heap objects; a dedicated test guards that property.

## Remaining boundaries

This adds `format-message`, not the separate `text-quoting-style` function.
The initial `internal--text-quoting-flag` is currently true, not locale-derived.
GNU initializes it from `using_utf8()` in `src/emacs.c`, which checks the active
C locale by decoding UTF-8 U+0100. Non-UTF-8 startup locales therefore remain a
known difference; explicit dynamic bindings of the flag are supported.

The shared formatter still supports only `%s`, `%S`, `%d`, and `%%`; numeric
widths/precision, other conversions, full text-property propagation, and string
identity optimizations are outside this slice. Ordinary `format` retains its
pre-existing mixed-raw-octet/Unicode error path. `message` is not rerouted here.
