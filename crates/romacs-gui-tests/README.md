# Independent GUI test support

Keep this crate's workspace membership. It no longer discovers or launches the
retired `romacs` executable and has no MELPA support dependency.

Retained:

- Display-session ownership/authentication contracts and generic subprocess,
  artifact, manifest, timeout/readback, and font-probe contracts.
- Native Linux/macOS desktop input/capture adapters and coordinate mapping tests.
- The standalone winit/wgpu resize/present contract (no editor or redisplay).
- The raw Wayland subsurface-scale diagnostic example.
- GNU GUI Emacs live-desktop-font, width-resize, and pixelwise-geometry oracles,
  including their existing assertions and required fixtures.
- The GNU-compatible labeled font-selection probe and its structural contract.

Removed: legacy-editor GUI integration scenarios and exclusive fixtures;
third-party Clatter, Telega, SVG package and Vertico integration infrastructure;
the Telega mock executable/protocol and package-default command builder.
`GuiTestPlan` now defaults to `emacs`; callers may supply another program with
`with_program`. Historical artifact names and observation environment keys remain
for schema compatibility. Display provisioning still uses `romacs-infra::display`.

There are no VM2-specific tests/dependencies in this crate. The 35 retained test
functions include native-platform tests that require real display/tool/font
prerequisites.

## Linux prerequisites

The tests launch their own displays; an existing desktop session is not a
substitute for these tools on the **test process's** `PATH`:

- `Xvfb`, `xauth`, and `xdpyinfo` for authenticated TCP-only X11 sessions.
- `weston` for the owned headless Wayland session contract.
- `glib-compile-schemas` and `gsettings` for the private keyfile settings backend.
- ImageMagick `import` for the resize/present screenshot contract.
- A working wgpu adapter (software Vulkan is sufficient for the present test).

The live-font assertions require the real **Ubuntu Mono**, **Ubuntu**,
**DejaVu Sans Mono**, and **DejaVu Sans** fonts. Installing font files alone is
not enough: the isolated test environment must expose them through fontconfig.
The desktop-font tests require `ROMACS_GNU_GUI_FONTCONFIG`, a private
fontconfig file listing the chosen font directories and a writable private
cache directory, and apply it as `FONTCONFIG_FILE` only to their GNU GUI
subprocesses; without it they refuse rather than read the host's fonts. The
gate sets it from `ROMACS_GATE_FONTCONFIG`. Xvfb, xauth, xdpyinfo and weston
come from `ROMACS_DISPLAY_TOOLS` when it is set, as in the gate, else from PATH.
Do not inherit personal font substitutions. Check both
`fc-match 'Ubuntu Mono'` and `fc-match 'DejaVu Sans Mono'` with the chosen
`FONTCONFIG_FILE`. The expected metrics use 96 DPI, set by the test's X resource
argument.

`ROMACS_GNU_GUI_BINARY` selects the GNU GUI Emacs executable (default `emacs`).
Toolkit selection matters: PGTK running on X11 can coalesce pending resize
requests with a font change differently from a native X11 build. A failure of
`gnu_overlapping_resize_and_font_change_preserve_latest_grid` must remain visible;
do not add a wait between its overlapping requests or relax its final grid
assertion to make a toolkit pass. A passing run with another GNU version/toolkit
does not qualify the configured GNU reference executable.

The workspace gate constructs its own `PATH` and isolated environment. Wrapping
it in `nix shell` or exporting `FONTCONFIG_FILE` in the caller does not provision
those inputs unless the gate explicitly carries them into its test environment.

Run with the appropriate display/GNU prerequisites when builds are permitted:

```sh
cargo test -p romacs-gui-tests --test interaction_geometry
cargo test -p romacs-gui-tests --test harness_contract
cargo test -p romacs-gui-tests --test desktop_font_updates
cargo test -p romacs-gui-tests --test frame_resize_oracle
cargo test -p romacs-gui-tests --test display_window_geometry
cargo test -p romacs-gui-tests --test present_contract
```
