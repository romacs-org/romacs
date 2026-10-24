# VM2 GUI

`scene.el` is evaluated as source in VM2. VM2 publishes owned Seam L snapshots;
`romacs-layout2` produces Seam P paints and resolved font bindings. `PaintHost`
owns the native loop and renderer. External stdin JSON-line requests go through
`Vm::submit`. Changes, including changes preceding an evaluation error, trigger
a new snapshot and paint; unchanged state retains its presentation. Native
notices are drained while stdin waits. stdout contains JSON replies after
presentation activation.

## Build and test

Run from the repository root:

```sh
cargo test -p romacs-layout2 --test layout
cargo test -p romacs-vm-gui --test font_diagnostic -- --nocapture
cargo build -p romacs-vm-gui -p romacs-gui-oracle
G1A_ORACLE="$PWD/target/debug/romacs-gui-oracle" \
  python3 -m unittest discover -s crates/romacs-vm-gui/harness -v
```

The live harness needs a compatible compositor, capture tools and the configured
font. Use a new output directory for each run:

```sh
python3 crates/romacs-vm-gui/harness/live_oracle.py \
  --binary "$PWD/target/debug/romacs-vm-gui" \
  --oracle "$PWD/target/debug/romacs-gui-oracle" \
  --output "$PWD/target/evidence/gui-live"
```

Build the acceptance executable package-scoped. Cargo features are additive:
a workspace build may unify `legacy-core` into display-runtime. Check the actual
executable with `harness/check_graph.py --binary PATH --output NEW.json`.

## Coverage and limits

The calibrated scene covers text and colour, an external edit, and normal,
low-floor and explicit-moving-GC presentation. The pixel oracle rejects wrong
colour, a one-pixel shift and a missing Delta. Identical glyph pairs in the font
cannot establish Unicode identity. Collection counts, buffer versions, font
projection and renderer counters are diagnostics, not substitutes for pixels.

The harness's synthetic-font test checks the judge and mutation wiring, not a
live GUI. A startup failure likewise is not a pixel-behaviour failure. Historical
run captures and logs are not shipped fixtures. Maintained Lisp inputs and
harness code remain under `evidence/`; send new run outputs to `target/evidence/`.
