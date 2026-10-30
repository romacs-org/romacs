# Live VM G1a oracle harness

Run from the repository root **after the parent has built both binaries**:

```sh
python crates/romacs-vm-gui/harness/live_oracle.py \
  --binary /absolute/path/to/romacs-vm-gui \
  --oracle /absolute/path/to/romacs-gui-oracle \
  --output /absolute/path/to/new-artifact-directory
```

The harness never invokes Cargo. The output directory must not exist. Defaults
use the already-installed private labwc and wlr-randr from the oracle capture
harness; override `--labwc` and `--wlr-randr` if those store paths change.
`grim`, ImageMagick (`magick`), and the Liberation Mono Regular font file must
be available. Python uses only the standard library. `--runtime-root` must be
short enough for Unix sockets; its default is the supplied Hermes scratch root.

## Binary contract and independent geometry

* Each run receives `--wayland /absolute/private/socket`. The compositor has
  **one 400×300 headless output**, configured explicitly with wlr-randr. Its
  window rule maximizes the client without decorations. Capture the whole
  output; there is **no inferred crop, image alignment, or renderer geometry**.
* Startup is `Ωμέγα Hello\n`, with only `Hello` red `#ff0000`, default white
  `#ffffff`, background black `#000000`, no cursor/fringe, origin **(0,0)**.
  Font: **Liberation Mono, 20px**.
* The binary writes exactly one JSON object per stdout line, with no log lines:
  `{"ready":true,"presentation":1}` after startup presentation activation.
* Only after capturing and judging the initial frame, the harness sends this
  stdin JSON line (the actual JSON encoder preserves Unicode):
  ```json
  {"source":"(with-current-buffer \"*g1a*\" (goto-char (point-max)) (insert \"Δ\"))"}
  ```
* The reply must contain `result` and a strictly newer positive integer
  `presentation`, **after activation**. `error` is forbidden. The harness then
  captures the second frame, which must have `Δ` at row 1, column 0.
* Replies are ordering signals only, never inputs to the pixel judge.
  ImageMagick's independent FreeType measurement of the declared font file at
  density 72 and point size 20 determines cell width, ascent, and row height
  `ceil(ascent + abs(descent))`. The installed Liberation Mono measures
  **width 12, ascent 17, descent 6, row height 23**. Thus the two baselines are
  y=17 and y=40. These metrics are recorded and explicitly passed in each
  oracle spec. Neither capture content nor renderer reports tune them.

The harness runs three fresh compositor/client pairs:

1. normal, no GC arguments;
2. `--gc-floor 1` (override the floor with harness `--gc-floor N`);
3. `--gc-floor 1 --gc-between-snapshot-and-paint`.

Every run judges before and after captures using `oracle check SPEC CAPTURE`
(exit **0** required). Against the *after* spec it also requires exit **1**,
not an error exit, for three mutants derived from that run's real captures:

* red Hello ink changed to green, preserving antialias coverage;
* the whole captured image shifted right by exactly one pixel on black;
* the actual before capture substituted, so `Δ` is absent.

Low-GC and forced-GC captures must additionally match normal-run RGB pixels
exactly for each phase. A successful result is `summary.json` with `pass: true`
and three successful runs. Failure writes `pass: false` plus the error and
retains diagnostics. Each run saves launch arguments, replies, request,
compositor/client logs, captures, mutants, and every oracle result. Runtime
HOME/XDG directories and sockets are disposable; no ambient DISPLAY or
WAYLAND_DISPLAY is inherited. Processes are terminated (then killed on timeout)
on all paths.

The text claim is **exact up to Liberation Mono's identical glyph pairs**;
`H/Η` and `o/ο` occur in this scene. GC counters, buffer versions, and unresolved
font counters are not pixel assertions. The binary's forced-GC implementation
and core-free dependency/symbol checks remain the parent's responsibility.

## Focused verification, without Cargo

```sh
G1A_ORACLE=/absolute/path/to/romacs-gui-oracle \
  python -m unittest discover -s crates/romacs-vm-gui/harness -v
```

Without `G1A_ORACLE`, the real-oracle unit test is explicitly skipped. It renders
an **explicitly synthetic ImageMagick test image** to verify independent metrics,
spec wiring, and all three actual oracle rejections. It is **not** a live
compositor/editor acceptance result or a substitute for running the command at
the top.

Implementation-time checks:

* Red-first: unit discovery initially failed because `live_oracle` did not yet
  exist (an import failure, **not** a behavioral red acceptance test).
* All **6 focused tests passed** with the existing s107 debug oracle, including
  the synthetic positive-before/after and three negative controls.
* The private compositor path was exercised with `/usr/bin/false` deliberately
  used as the client. wlr-randr configured the output and client launch was
  reached; the harness correctly returned exit 1 / `pass: false` with
  `binary stdout closed (exit=1)`. This is only a fail-closed launch preflight.
* The integrated worker subsequently built the real binary and ran all three
  live variants successfully; see `../evidence/current-oracle-live/summary.json` and
  `../README.md` for the exact commands, movement proof, and diagnostic limits.
  This harness does not invoke Cargo, a full gate, or Git publication.

# Package parity: pinned GNU beside `--loadup`

`parity_live.py` runs pinned GNU pgtk and `romacs-vm-gui --loadup` at once,
each in its own headless labwc with a private HOME, types the same keys into
both and compares the state both report after every step through
`parity_probe.el`; `report.json` records every difference, the geometry
apart, and each step's wall time and, with `--perf`, its `instructions:u`.
It never invokes Cargo.  Each harness names its workflows, its private trees
and its comparison policy in its docstring:

* `gnosis_parity_live.py`: the owner's gnosis workflows (dashboard, search
  and tags, review, adding and editing a thema), each engine on its own copy
  of a Gnosis database.
* `hermes_parity_live.py`: the owner's hermes checkout, read-only against
  the running dashboard at 127.0.0.1:9119 (dashboard, session browser and
  search, a session's history), its token discovered by hermes and never
  reported.

# Paired-session bench: agents drive GNU and Romacs side by side

`bench/bench.py` is an MCP server for agents.  `bench_start` launches pinned
GNU pgtk and `romacs-vm-gui --loadup`, each on a headless labwc with a private
HOME, both on the owner's session (`bench/owner.py`: init.el's editing,
completion (fido-vertical), org, dired, project, vc, vc-plus, eat, eshell,
gnosis and hermes blocks, without straight).  Each engine serves
`rlisp/romacs-mcp.el` with the session tools from
`rlisp/romacs-mcp-session.el`.  The same Lisp runs in both engines, so both
expose the same tools with the same result shapes:

* `romacs_send_keys`: `kbd` keys or literal text, fed through
  `unread-command-events`.  By default it waits for each key to be consumed
  and the command loop to go idle, like typing; `burst` queues all keys at
  once.
* `romacs_wait_idle`: answers once input is drained, the loop has waited for
  input `settle` seconds, no one-shot timer is due within `horizon`, and no
  process changed status or buffer for `settle` seconds, or at `timeout`.
  It says which of these held.
* `romacs_observe`: the selected window and buffer, the window tree, and
  each window's start, point, mode and header lines and displayed rows.
  Invisible text is dropped; display and overlay strings are included, so
  icomplete's candidates show.  Face runs are optional.  Also the
  minibuffer, the echo area, `*Messages*` lines since a cursor, the last
  command-loop error, an optional backtrace, the modes and the buffer list.

Both tools that wait answer later through `romacs-mcp-defer`, so the command
loop keeps running while they wait.  The bench starts the endpoint with
`romacs-mcp-start-loopback`: 127.0.0.1 on an ephemeral port, and the peer's
first line must be a per-session token.  Romacs's GUI host refuses local
network processes, so `romacs-mcp-start` cannot listen there.

The bench's own tools take `engine` (`gnu`, `romacs` or `both`):
`bench_start`, `bench_send_keys` (events or wtype), `bench_wait_idle`,
`bench_observe`, `bench_eval`, `bench_screenshot`, `bench_compare` (verdict
same / differs / romacs-error with the differences, under the policy in
the docstring) and `bench_stop`.  Every call is appended to the session's
`recording.jsonl`, and its full result to `results.jsonl`.

Data safety: the project is a `git clone --no-hardlinks` copy and the org
file a copy, both in the private HOME.  The gnosis database is a reflink copy
of the copy named by `--gnosis-db`, never the live database.  Hermes attaches
to the running dashboard at 127.0.0.1:9119.  A key that sends a hermes prompt
must carry `effect=hermes-prompt`.  The bench allows one per engine per
session, and replay skips such keys unless `--allow-effect hermes-prompt` is
given.

```sh
source ~/.cache/claude-scratch/bin/romacs-env.sh   # pinned GNU, ROMACS_GNU_SOURCE
export ROMACS_BENCH_ROMACS=$CARGO_TARGET_DIR/gate/romacs-vm-gui
export ROMACS_BENCH_OUTPUT=~/.cache/romacs-bench ROMACS_BENCH_GNOSIS_DB=/path/to/copy/gnosis.db
export ROMACS_BENCH_ORG=/path/to/notes.org   # copied to ~/Documents/<its name> in each private HOME
# Claude Code
claude mcp add romacs-bench -e ROMACS_BENCH_ROMACS -e ROMACS_GNU_ORACLE -e ROMACS_GNU_SOURCE \
  -e ROMACS_BENCH_OUTPUT -e ROMACS_BENCH_GNOSIS_DB -e ROMACS_BENCH_ORG -- python3 -B "$PWD/crates/romacs-vm-gui/harness/bench/bench.py" mcp
# Codex: ~/.codex/config.toml
#   [mcp_servers.romacs-bench]
#   command = "python3"
#   args = ["-B", "/path/to/romacs/crates/romacs-vm-gui/harness/bench/bench.py", "mcp"]
#   env = { ROMACS_BENCH_ROMACS = "...", ROMACS_GNU_ORACLE = "...", ROMACS_GNU_SOURCE = "...",
#           ROMACS_BENCH_OUTPUT = "...", ROMACS_BENCH_GNOSIS_DB = "...", ROMACS_BENCH_ORG = "..." }
#   tool_timeout_sec = 600
# Without an MCP client: a daemon and one call at a time
python3 -B crates/romacs-vm-gui/harness/bench/bench.py daemon --control $XDG_RUNTIME_DIR/bench.sock
python3 -B crates/romacs-vm-gui/harness/bench/bench.py call --control $XDG_RUNTIME_DIR/bench.sock \
  bench_send_keys '{"keys": "C-x C-f", "wait": 10, "step": "find-file"}'
# Re-drive a recording against a new binary, the GNU side cached from a run
python3 -B crates/romacs-vm-gui/harness/bench/bench.py replay \
  crates/romacs-vm-gui/harness/bench/recordings/daily-journey.jsonl \
  --output OUT --gnu-cache RUN/session-*/results.jsonl
```

The first time `bench_start` runs for a package set, the pinned GNU
byte-compiles that set, which takes minutes.  After that, a start takes
about 10 s for GNU and 25-35 s for Romacs.  `recordings/daily-journey.jsonl`
is the first recorded agent session, the owner's daily journey: find-file
and minibuffer motion, editing, project commands, org, a gnosis review,
one hermes project-chat prompt, help and search commands, and quitting.
It has 103 compared steps.  Replaying it with the GNU side cached takes
about 5 minutes.  It keeps observations out of the repository: only the
calls and their verdicts are recorded, and a run's `results.jsonl` stays
local.  Gnosis due cards and model replies depend on the day, so on
replay those steps can diverge for reasons outside Romacs.
