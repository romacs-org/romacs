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
* `hermes_parity_live.py`: hermes (lisp/romacs's copy, or a profile config's
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
* `theme_scenes_live.py`: standard scenes (code with a region and line
  numbers, org, help beside a diff, Emacs Lisp beside dired, a toast, the command
  panel, and notmuch hello/search/show on `notmuch_fixture.py`'s store
  when the system has notmuch) under each of `--themes`, one private
  session per theme, PNGs plus one sheet per scene stacking the themes;
  `--gnu` puts pinned GNU under the reference Rum configuration first.
  For judging a theme by eye, not a gate.
* `input_method_live.py`: input methods by real keys against pinned GNU:
  greek-polytonic's dead keys and C-\\, a pending sequence's overlay, C-g and
  DEL, bulgarian-phonetic, TeX, chinese-py's candidates and completions,
  korean-hangul, `describe-input-method`, INHERIT-INPUT-METHOD, C-q,
  `read-passwd`, isearch and keyboard macros.
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
with `-l` (Romacs as `--loadup -Q -l`).  Both use only the stock lisp/gnu and lisp/romacs, on a
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
`lisp/romacs/romacs-mcp.el` with the session tools from
`lisp/romacs/romacs-mcp-session.el`.  The same Lisp runs in both engines, so both
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

`perfbench/perfbench.py` runs one fixed set of workloads in GNU Emacs and
in a given Romacs build and prints one table: per workload, each engine's
`instructions:u` (the reliable metric) and wall time (median of `--runs`,
default 3), the Romacs/GNU ratio of each, and the change of Romacs's
figures since the previous saved run.  Each run is saved as
`results.json` and `table.md` under `OUT/<commit>/<UTC time>/` (default
OUT `~/.cache/romacs/perf-bench`, or `$ROMACS_PERFBENCH_OUT`), so a rerun after every master
shows the trend.

The GNU engine is this machine's own installation, `/usr/bin/emacs`, run with
`LD_LIBRARY_PATH` unset, or `--gnu BIN`.  Each run records its version,
repository commit, package and whether it is natively compiled, in results.json
and README's headers, since an upgrade moves it.  The parity harnesses keep the
pinned oracle.  `$ROMACS_GNU_SOURCE` still supplies the pinned tree: fixture
files and ERT test files, the same inputs for every engine, whatever commit the
GNU engine is at.

Romacs runs as an installation runs it.  Before anything is measured,
perfbench stages the binary beside its Lisp trees and compiles them with
`--compile-lisp-tree`, as an installer does, into a store of its own,
cached under OUT per binary and runtime (minutes the first time, not part
of any row).  Batch rows set `ROMACS_COMPILED_LISP=on` and boot from a
batch world perfbench prepares first; a user's is built by their first
batch run once their cache holds a world store, so the very first batch
run of an installation is slower than these rows.  GUI rows have the store
and their world by default.  A workload's own file (micro, elb) loads by
name with an `.elc` beside it, as an installed package does.  A load the
store cannot satisfy reads source without saying so, so each row's note
counts the loads the store's use log confirms, names any read as source,
and says if anything compiled during the row.  Perfbench's own Lisp, its
init and ERT's test files read as source in both engines, and a world
build reads source by design.  The init and apps rows load private copies
of their packages, which read as source, and their notes say so.
`--romacs-source` measures the old way, with no store or world.

`--neomacs BIN` adds neomacs as a third engine with its own column: the same
workloads in private homes, as it installs, its dump and its `.elc` files,
with a workload's own file byte-compiled by its own compiler.
`--neomacs-commit SHA` records the commit it was built from, which it does
not report itself.  Its batch runs are limited to 8 GiB of address space,
because one ERT file otherwise grows without bound.  A row it cannot run says why.  The `init` and `apps`
rows are GNU's and Romacs's alone, and so is ERT.

The workloads, and how each is counted, are in the script's docstring:

* batch, `-Q --batch` on `perfbench/perfbench.el` in both engines:
  startup; font-lock-ensure on fileio.c, subr.el and ORG-NEWS; dired of
  5,000 files; project-find-regexp over the Emacs repository; vc-log of
  2,000 commits; occur and how-many over xdisp.c; find-file of xdisp.c;
  describe-function of find-file; 20 shell-command-to-string and 20 eshell
  commands.  Instructions are a run's
  less a run doing only the setup (perf/font-lock's method).  Each engine's
  result (match counts, line counts) is compared, and a difference is noted.
* micro: named call, closure call, mapcar of 1 and 10, dolist + funcall,
  per iteration beyond an empty loop; GNU byte-compiled, Romacs from its store.
* elb: GNU ELPA's elisp-benchmarks 1.16 (a pinned tarball, checked by
  SHA-256), the suite GNU evaluates native compilation with.  One row per
  entry point the suite defines in either engine, and their geometric
  mean.  GNU loads the member files byte-compiled (JIT native compilation
  off) and, when it has native compilation, natively compiled at the
  suite's own speed and safety, a third column; Romacs loads them from its
  store.
  A member an engine does not define, or whose run reported an error,
  has no figure there and says why.  The geometric mean compares Romacs
  with GNU `.elc` alone.  A median of the per-member ratios covers every
  column: native compilation reduces some members to almost nothing, and one
  such ratio would decide a mean.  Pure Lisp compute rather than an editor
  workload, so it has its own table and is not in `--quick`.
* ERT: 25 GNU ERT files with the same `-l ert ... -f
  ert-run-tests-batch-and-exit` command in both engines.  These totals are
  not comparable with `test/gnu/run.py`'s, which run romacs-gnu-ert
  through the romacs-batch adapter.
* session, real commands and keys in a GUI session on headless labwc:
  dired, occur, project-find-regexp, project-find-file, vc-log, vc-diff,
  vc-dir, `M-x shell` and org-agenda on a key, and per-key kill, yank, undo,
  isearch and Org editing.  Then, in a session with Romacs's shipped defaults
  on, the rows README's lead quotes: find-file of `xdisp.c` and of a generated
  15 MB header to the window's final faces (every visible character
  `fontified`), `vc-print-root-log` over upstream master's whole history with
  `vc-log-show-limit` 0 (commits, complete, first screen, median key while it
  streams, the buffer's hash compared across engines), and a theme switch,
  cycled and on first visit, among each engine's own shipped themes, since
  Romacs ships none of GNU's.  Wall runs from the key to the result.
  Block is the longest gap between ticks of a 5 ms timer, which cannot run
  while the main VM is busy (`perfbench-session.el`).  Instructions are
  reported, but these rows are read by wall and block: a worker VM's path does
  more work in total to keep the main VM free, and its processes are not
  counted.  Writing rows use a private clone of the pinned tree.
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
* `init`, by name only (`--only init`): the real-init profile's init body (`--profile-config`), GNU against Romacs from
  source and from its init layer; wall only.
* `apps`, by name only (`--only apps`): gnosis dashboard open and back (`gnosis_parity_live.py` on a database
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
# in the project environment (ROMACS_GNU_SOURCE for the pinned tree)
python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py \
  --romacs INSTALLED/romacs-vm-gui  # about an hour, most of it elb
python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py --romacs BIN --quick  # about 4
```

After a full run, `perfbench/readme.py OUT/COMMIT/TIME/results.json` rewrites the
tables in `doc/benchmarks.md` from that run, keeping every
row: the editor workloads between `<!-- perfbench-table-begin -->` and
`<!-- perfbench-table-end -->`, and the elisp-benchmarks members between
`<!-- perfbench-elb-begin -->` and `<!-- perfbench-elb-end -->`.  A block the run did not
measure is left as it was.  `perfbench/lead.md` is README's lead: each `{{name}}`
in it is filled from the run and written between `<!-- perfbench-lead-begin -->` and
`<!-- perfbench-lead-end -->`.  A name with no value fails the run and stops `readme.py`
with that README untouched, and a wall time refused for load counts as none, so
the lead takes a quiet run.  A whole run also rewrites the headlines between
`<!-- perfbench-headlines-begin -->` and `<!-- perfbench-headlines-end -->`: every row where
Romacs is 0.80x or better against a rival on every metric measured, with build
and rerun command.  It names on standard error any headline the run lost.

A binary installed beside its runtime (`lisp/gnu/`, `lisp/romacs/`, as an
installed build has them) runs that runtime; otherwise give
`--runtime`.  The commit comes from the `build.json` beside the binary, or
from `--commit`.  `--only` takes groups (`batch micro ert elb session gui init apps`)
or workloads: a batch row's name, `micro`, `ERT 25 files`, `elb`, `session`, `launch`,
`keys`, `init` or `apps`.

## Line numbers in programming buffers

An interactive session's defaults (`romacs-defaults-mode`, without `-Q`)
number programming-mode buffers' lines relatively. The digits are ordinary
text glyphs in the scene diagnostics, so a harness that matches a
programming buffer's rows by their glyph text starts its init with
`live_oracle.NO_LINE_NUMBERS`, which turns them off the way a user's init
does (`fontify_live.py`, `lines_live.py`).

## The render thread at rest (`rest_live.py`)

What the render thread draws, and why, while nothing on screen changes.
Under `ROMACS_DIAGNOSTICS=1` it writes its frame counters as `frame-stats`
lines on stderr (`render_thread/frame_stats.rs`): frames planned and
presented per demand reason (`redisplay` for dirty content, `platform_redraw`
for a redraw nothing of ours asked for, an effect's own name beside
`redisplay` while it animates), redraw requests and events, the scenes the
evaluator sent (`scene_commits`) and loop wakeups; one line a second while
they move, none once they rest, so the last line holds the totals. The
harness holds one `--loadup` session in four states by real keys and takes
each state's frames from those lines: idle with the blink off (no frame,
the main thread under 1% of a core), GNU's blink (two frames a second), an
agent chat whose turn runs (its clock, one frame a second) and the same
with a timer 20 times a second that changes nothing. A frame that answers
no scene of the evaluator's, or names another reason, fails its state, and
so does a blink or a clock that draws too little for the time it was held.
`ROMACS_HARNESS_WLR_ENV` (`KEY=VALUE ...`) is added to the session's
environment to run the private compositor on a real GPU, where effects the
software renderer suppresses can stand.

```sh
python3 -B crates/romacs-vm-gui/harness/rest_live.py --binary BIN --output DIR \
  [--seconds 3..6] [--tools LABWC-DIR]   # about 1 minute
```

## Typing while background work allocates (`gc_typing_live.py`)

Key latency against collections: one isolated `-Q --loadup` session with
org, eshell, dired and cc-mode loaded, a timer allocating as an agent's
stream does (`--rate` heap units a second), and 400 keys typed in bursts
with pauses (a fixed schedule from `--seed`), into the visited file or,
with `--prompt`, into a `read-string` prompt. A key's latency runs from
just before it is typed to its command's end, by the wall clock; a key met
a collection when it overlaps one (`gcs-done` and `gc-elapsed` from a
`post-command-hook`) and took 50 ms or more. It reports the latency
percentiles, the collections, the keys that met one and the worst of
those, and refuses a run whose keys and records do not match. `--setup`
evaluates more Lisp with the setup, such as a variant's
`romacs-gc-idle-delay`.

```sh
python3 -B crates/romacs-vm-gui/harness/gc_typing_live.py --binary BIN --output DIR \
  [--prompt] [--seed N] [--tools LABWC-DIR:WLR-RANDR-DIR]   # about 3 minutes
```

## Typing while jabber catches up its archives (`jabber_live.py`, `xmpp_fixture.py`)

`xmpp_fixture.py` is a private XMPP server on 127.0.0.1 (standard library
Python): SCRAM-SHA-1, bind, roster, disco, private storage, MUC joins with
every occupant's presence and live traffic in busy rooms, and MAM with
RSM paging (`start`, `after`, `before`, `with`; pages clamped to 50) over
generated archives in Prosody's result format, from a seed and an anchor
time. Live room messages enter their room's archive. It logs every MAM
request and counts the client's presences and messages.

`jabber_live.py` runs one isolated `--loadup` session with a notmuch
search (notmuch_fixture.py) above a text file, connects jabber to the
fixture, and types bursts of C-n/C-p and letters (F5/F6 select the two
dedicated windows) while the archives catch up, then waits until every
expected archive settled, no job runs or waits and every buffer refresh
finished. Per key it records latency (wtype to `post-command-hook`) and
the spans of jabber's main-VM work (FSM events, jabber timers, isolate
deliveries, the background sync's steps, buffer refreshes, the session's
SQLite statements): a slow key is *associated* with jabber when a span
overlaps it and *attributable* when the spans' union within the key covers
its excess over `--slow` (16 ms). Variants: `session` (in-session
catch-up, `--page`), `worker` (`jabber-mam-background`), `none` (noise
floor). `--restricted` makes the live connection read-only (no presence,
no joins, a send gate); `--push-bookmark` has the fixture push an autojoin
bookmark, which must not lead to a join. `--chats open` shows the next
chat or room buffer (F7) every third window switch; `--chats send` also
sends a message from it (F8; fixture account only), and the report counts
the sends the fixture received, the background sync's reports to the
session (`first_post_s` after connecting) and duplicates after the
handover: stored, shown, and live messages stored more than once. The run
fails on an unfinished catch-up, a failed archive, no worker, unpaired
keys, a refresh error, a duplicate or a lost send or, in the worker
variant, a key during the catch-up attributable to the sync's work.

```sh
python3 -B crates/romacs-vm-gui/harness/jabber_live.py --binary BIN --output DIR \
  --variant worker [--backlog 300] [--resync-after 25] [--duration 60] \
  [--chats open|send]   # about 3 minutes
```

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
