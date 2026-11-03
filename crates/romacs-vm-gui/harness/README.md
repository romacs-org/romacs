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

* `gnosis_parity_live.py`: everyday gnosis workflows (dashboard, search
  and tags, review, adding and editing a thema), each engine on its own copy
  of a Gnosis database (`--db`, or a profile config's; a live database is
  refused).
* `hermes_parity_live.py`: hermes (rlisp's copy, or a profile config's
  `hermes_sources`), read-only against a running dashboard (hermes's
  default URL, or the profile's `hermes_url`), never started (dashboard, session browser and
  search, a session's history), its token discovered by hermes and never
  reported.
* `notmuch_parity_live.py`: the real notmuch package (the system's
  site-lisp) through a user's mail settings as a Romacs init holds them
  (`--snippet`): notmuch-hello, a saved search, tagging and archiving,
  notmuch-show with HTML and attachments, notmuch-tree, composing with
  address completion, an attachment and a saved draft, and sending a new
  mail over STARTTLS and a reply over implicit TLS.  Each engine has its
  own copy of a synthetic store (`notmuch_fixture.py`) and its own
  loopback SMTP server (`fake_smtp.py`); the captured submissions must
  agree.
* `command_errors_live.py`: the command loop and the session's end on a
  plain init: command errors' echo area, *Messages* and bells, the
  debugger's *Backtrace* from M-: and from a `debug-on-error` command,
  keyboard macros through kmacro.el's keys, C-x k's prompts, auto-saves by
  event count and idle time, SIGTERM, and C-x C-c over a modified file
  through the save prompt to the exit status.
* `eshell_parity_live.py`: `M-x eshell` from *scratch* through Eshell's own
  `ls`, `echo`, `cd` and `pwd`, `cat` of a file several screens long, an
  external `printf`, `C-c C-c` on an external `sleep 5`, `M-p`, `C-c C-u`
  and `exit`, both engines in one small work directory.  `--debug-on-error`
  sets `debug-on-error` in both inits.

# Paired-session bench: agents drive GNU and Romacs side by side

`bench/bench.py` is an MCP server for agents.  `bench_start` launches pinned
GNU pgtk and `romacs-vm-gui --loadup`, each on a headless labwc with a private
HOME, on one of two profiles (`real_init.py`).  The default, `stock`, loads
no one's init: both engines run `-Q` and load only the bench's endpoint
with `-l` (Romacs as `--loadup -Q -l`).  Both use only the stock gnu-lisp and rlisp, on a
generic project at `~/src/tally` (`bench/fixture.py`: Emacs Lisp, C, Rust,
Markdown, Org and a Makefile, with a five-commit history whose authors and
dates are fixed, so the hashes match everywhere).  `init` is a real-init
profile, described by a JSON profile config kept outside the repository
(`--profile-config FILE` or `ROMACS_BENCH_PROFILE`): the init's blocks, the
package sources it loads, the project to clone, an org file, a gnosis
database copy and the hermes dashboard URL; `real_init.py`'s docstring
lists the keys.  Both
profiles give GNU the settings Romacs's `--loadup` makes by design, as the
parity harnesses do, and Romacs GNU's completion: fido-vertical, not its
own completion layer (`romacs-complete-mode`), which takes the root
window's last line, and the completion styles without fzf.  Each engine serves
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
of the copy named by `--gnosis-db` or the profile config, never the live
database: gnosis's default locations and the profile's `live_gnosis_db` are
refused.  Hermes attaches to a running dashboard and never starts one: an
init that sets another start mode is refused, and every acquisition is made
remote.  A key that sends a hermes prompt
must carry `effect=hermes-prompt`.  The bench allows one per engine per
session, and replay skips such keys unless `--allow-effect hermes-prompt` is
given.

```sh
# in the project environment (pinned GNU, ROMACS_GNU_SOURCE)
export ROMACS_BENCH_PROFILE=/path/to/profile.json   # only for the init profile
export ROMACS_BENCH_ROMACS=$CARGO_TARGET_DIR/gate/romacs-vm-gui
export ROMACS_BENCH_OUTPUT=~/.cache/romacs-bench ROMACS_BENCH_GNOSIS_DB=/path/to/copy/gnosis.db
export ROMACS_BENCH_ORG=/path/to/notes.org   # copied to ~/Documents/<its name> in each private HOME
# Claude Code
claude mcp add romacs-bench -e ROMACS_BENCH_ROMACS -e ROMACS_GNU_ORACLE -e ROMACS_GNU_SOURCE \
  -e ROMACS_BENCH_PROFILE -e ROMACS_BENCH_OUTPUT -e ROMACS_BENCH_GNOSIS_DB -e ROMACS_BENCH_ORG -- python3 -B "$PWD/crates/romacs-vm-gui/harness/bench/bench.py" mcp
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
  crates/romacs-vm-gui/harness/bench/recordings/general-usage.jsonl \
  --output OUT --gnu-cache RUN/session-*/results.jsonl
```

A `stock` start takes about 4 s for GNU and 17 s for Romacs.  The first
`init` start for a package set has the pinned GNU byte-compile that set,
which takes minutes; after that it takes about 10 s for GNU and 25-35 s for
Romacs.

`recordings/general-usage.jsonl`, on the `stock` profile, is general use of
the machine:
* find-file across directories, with minibuffer editing, completion and
  history;
* project.el: find-file, find-regexp, dired, switch-to-buffer, compile,
  eshell, shell-command, query-replace and vc-dir with its log and diff;
* dired copy and rename, vc-annotate and vc-log;
* help (`C-h f`, `C-h v`, `C-h k`, `C-h m`, apropos, Info);
* isearch, occur, grep, eval-buffer and `M-.`;
* editing: undo and redo, rectangles, registers, keyboard macros, fill,
  comment-dwim and indent-region;
* Org: cycling, TODO, a table and export to a buffer;
* splits, winner and tab-bar;
* `M-x` with completion, customize-variable, calc, calendar, IELM and
  `M-:`;
* quitting.

It has 94 compared steps; a replay with the GNU side cached takes about 5
minutes.  Every call drives both engines with the same keys: a Romacs
failure shows in the steps after it rather than being worked around, so
the recording needs no change when it is fixed.  A `--gnu-cache` must come
from a live run of this same recording, since compares are matched by
order.  It leaves out list-packages, which would need the network.

A recording made on the `init` profile names a user's own files and
projects, so it is kept beside the profile config, outside the repository,
and replayed the same way.  A recording keeps observations out: only the
calls and their verdicts are recorded, and a run's `results.jsonl` stays
local.  Gnosis due cards and model replies depend on the day, so on
replay those steps can diverge for reasons outside Romacs.

# Standing benchmarks: fixed workloads against pinned GNU

`perfbench/perfbench.py` runs one fixed set of workloads in pinned GNU and
in a given Romacs build and prints one table: per workload, each engine's
`instructions:u` (the reliable metric) and wall time (median of `--runs`,
default 3), the Romacs/GNU ratio of each, and the change of Romacs's
figures since the previous saved run.  Each run is saved as
`results.json` and `table.md` under `OUT/<commit>/<UTC time>/` (default
OUT `~/.cache/romacs/perf-bench`, or `$ROMACS_PERFBENCH_OUT`), so a rerun after every master
shows the trend.  The workloads, and how each is counted, are in the
script's docstring:

* batch, `-Q --batch` on `perfbench/perfbench.el` in both engines:
  startup; font-lock-ensure on fileio.c, subr.el and ORG-NEWS; dired of
  5,000 files; project-find-regexp over the Emacs repository; vc-log of
  2,000 commits; occur and how-many over xdisp.c.  Instructions are a run's
  less a run doing only the setup (perf/font-lock's method).  Each engine's
  result (match counts, line counts) is compared, and a difference is noted.
* micro: named call, closure call, mapcar of 1 and 10, dolist + funcall,
  per iteration beyond an empty loop; GNU byte-compiled, Romacs from source.
* ERT: 25 GNU ERT files with the same `-l ert ... -f
  ert-run-tests-batch-and-exit` command in both engines.  These totals are
  not comparable with `gnu-tests/run.py`'s, which run romacs-gnu-ert
  through the romacs-batch adapter.
* GUI, each engine on its own headless labwc as the parity harnesses run
  them: first-start world build (Romacs only), warm launch to first paint
  (Romacs only) and to idle, per-key self-insert, C-f and C-n, and C-x C-f
  over 5,000 files in fido-vertical, then (Romacs only) in its default
  completion, the `romacs-complete-mode` layer with the fzf style.  The
  comparison rows give Romacs GNU's completion, as the bench's profiles do
  (`real_init.GNU_COMPLETION_UI`): fido-vertical, not the layer, which takes
  the root window's last line, and the styles without fzf.  Per-key instructions count every
  thread but llvmpipe's: the headless compositor has no GPU, so Mesa's
  software GL does the GPU's work there.  The parity probe's polling is
  paused while a phase is counted.  Wall per key runs from
  `pre-command-hook` to the next idle moment, after redisplay
  (`perfbench-gui.el`).
* `init`, the real-init profile's init body (`--profile-config`), GNU against Romacs from
  source and from its init layer; wall only.
* gnosis dashboard open and back (`gnosis_parity_live.py` on a database
  copy given with `--gnosis-db` and `--gnosis-db-repo`, or the profile
  config's), and hermes
  dashboard open (`hermes_parity_live.py`, read-only, only when the
  dashboard answers), one run each.

When the 1-minute load average exceeds `--max-load` (default 8) during a
workload, its wall times are withheld and the load is printed beside them.
Instructions are still reported.  The inputs are pinned.  The Emacs tree
is a `--shared` clone of `$ROMACS_GNU_SOURCE` at the oracle's commit,
checked clean, and its fixture files are checked against SHA-256s.  The
5,000 files have fixed contents and mtimes.  Every engine runs in private
HOME/XDG/TMPDIR trees, and no workload writes a user's data.

```sh
# in the project environment (pinned GNU oracle and source)
python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py \
  --romacs INSTALLED/romacs-vm-gui --profile-config PROFILE.json  # about 15 minutes
python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py --romacs BIN --quick  # about 4
```

A binary installed beside its runtime (`gnu-lisp/`, `rlisp/`, as an
installed build has them) runs that runtime; otherwise give
`--runtime`.  The commit comes from the `build.json` beside the binary, or
from `--commit`.  `--only` takes groups (`batch micro ert gui init apps`)
or workloads: a batch row's name, `micro`, `ERT 25 files`, `launch`,
`keys`, `init` or `apps`.

# Driving Romacs through its MCP tools (`romacs_mcp_client.py`)

A live harness can drive a Romacs session the way an agent does, through
the session's own MCP endpoint, instead of wtype keystrokes and ad-hoc
diagnostics: evaluate Lisp, read and edit buffers, type keys through the
command loop, wait for idle, and read the frame as text. The client runs
the `romacs-mcp` relay beside the Romacs binary and speaks MCP over its
stdio (standard library only).

```python
from romacs_mcp_client import RomacsMcp

# A server_live.Server (or anything with .name, .env and .args.romacs).
with RomacsMcp.for_server(server, agent_tools=True) as mcp:
    mcp.call('romacs_eval', code='(+ 1 2)')                          # '3'
    mcp.call('romacs_buffer_read', name='*scratch*', start=1, maxChars=400)
    mcp.call('romacs_send_keys', keys='C-x b notes RET')
    mcp.call('romacs_wait_idle', timeout=5)
    mcp.call('romacs_observe', maxRows=20)                          # the frame's rows
    mcp.tools()                                                     # what it offers
```

- `call(name, **arguments)` returns the tool's result, JSON-parsed when it
  is JSON, and raises `ToolError` on a tool error; `tool(name, arguments)`
  returns the raw `(request id, text, is_error)`.
- `instance` is optional on every tool. `call` fills it (from
  `romacs_identity`) for tools that take it, so a call meant for a Romacs
  that has since restarted is refused rather than run on the new one.
- `agent_tools=True` loads `romacs-agent`, which registers the editing,
  session and delegation tools (`romacs_buffer_edit`, `romacs_file_write`,
  `romacs_visit`, `romacs_send_keys`, `romacs_wait_idle`, `romacs_observe`,
  `romacs_delegate_task`, `romacs_buffer_read`, ...); a bare session
  offers `romacs_identity` and `romacs_eval`.
- `RomacsMcp.connect(socket, relay)` reaches any endpoint; outside a
  harness, `romacs-mcp --server NAME` (default `romacs`) finds the server
  `romacs --server NAME` runs, so an external agent can be configured once:
  `{"command": "romacs-mcp", "args": ["--server", "romacs"]}`.

`fake_harness.py` uses this client for its tool calls, and
`agent_live.py --workflow tools` checks every tool the endpoint offers
through it: a tool added without a check in `tour()` fails that workflow.

## Agent sessions with real harnesses (`agent_live.py --workflow real`)

`real` runs a Claude Code session in an isolated Romacs that calls every
Romacs MCP tool once (each result checked) and delegates a tiny diff
review to a Codex child through `romacs_delegate_task`. It spends money,
so it is not a default workflow. Its models are parameters with cheap
defaults: `--claude-model haiku`, `--codex-model gpt-6-luna`,
`--codex-effort low`. Real reviews use Sol at high effort. The instances
use the real configuration directories for their logins:
`--claude-config` (default `$CLAUDE_CONFIG_DIR`, else `~/.claude`) and
`--codex-home` (default `$CODEX_HOME`, else `~/.codex`).
