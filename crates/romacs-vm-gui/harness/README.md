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
  live variants successfully; see `../README.md` for the exact commands, movement proof, and diagnostic limits.
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

# Standing benchmarks against pinned GNU

`perfbench/perfbench.py` compares one Romacs build with pinned GNU Emacs
(`$ROMACS_GNU_ORACLE`, or `--gnu BIN`) row by row, on what using an editor
feels like and on the Elisp speeds under it.  It is meant to run often: the
whole suite takes under 15 minutes on a quiet machine, and `--only` picks
rows, so a lane compares its own area in a minute or two.

```sh
# in the project environment (ROMACS_GNU_ORACLE, ROMACS_GNU_SOURCE)
python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py --romacs BINARY
python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py --romacs BINARY --only syntax,typing
python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py --romacs BINARY --only '*c-ts-mode'
python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py --list   # every row and its areas
```

`--only` takes areas, row names and globs, comma or space separated:

* `startup`: `-Q --batch` to exit; a GUI launch to Romacs's first paint and
  to the first idle moment after startup.
* `typing`: per key in a GUI session, self-insert, `C-f` and `C-n` in text,
  and self-insert in the middle of `xdisp.c` (c-ts-mode) and `subr.el`; and
  the syntax rows' key in the middle.
* `scroll`: per key, `C-v` down `xdisp.c` (c-ts-mode) and `subr.el`, each
  screen fontified as it comes.
* `find-file`: `C-x C-f` over 5,000 files in fido-vertical and in Romacs's
  own completion; `xdisp.c` and a generated 15 MB header opened in c-ts-mode
  to the window's final faces; dired of the 5,000 files.
* `completion`: the `C-x C-f` rows and `M-x` in fido-vertical.
* `font-lock`: the typing and scroll rows in code, the syntax rows' key and
  `font-lock-ensure`, and Org's `ORG-NEWS`.
* `syntax`: `syntax-ppss` after an edit, `scan-lists`, `forward-comment`, a
  key in the middle and `font-lock-ensure`, on `subr.el` in emacs-lisp-mode
  and `fileio.c` and `xdisp.c` in c-mode and c-ts-mode.  `xdisp.c`'s c-mode
  `font-lock-ensure` is left out: `fileio.c`'s says the same at a fraction of
  the time.  Also by file and mode: `c-ts-mode`, `xdisp.c`, ...
* `lisp`: the call path (named and closure calls, `mapcar`, `dolist` +
  `funcall`), uncompiled Lisp (less a run that only loads it), core primitives (hash tables, `assq`,
  `plist-get`, `format`, `concat`, `string-match`, `intern`, insertion), and
  three members of GNU ELPA's elisp-benchmarks (smie, eieio, pack-unpack).
* `search` (occur, project-find-regexp), `vc` (the whole history log),
  `theme` (a theme switch, cycled and on first visit), `commands`
  (describe-function, 20 `shell-command-to-string`).
* `elb`, by name: every member of elisp-benchmarks 1.16, the suite GNU
  evaluates native compilation with, and their geometric mean and median.
  The published tables, from a whole run, hold the three `lisp` members.

Instructions (`instructions:u`, every thread of the process) are the main
figure, wall time the median of `--runs` (default 3) beside them, each with
the Romacs/GNU ratio and the change of Romacs's figure since the newest earlier
saved run that has the row.  Each run is saved as `results.json` and `table.md` under
`OUT/<commit>/<UTC time>/` (OUT `~/.cache/romacs/perf-bench`, or
`$ROMACS_PERFBENCH_OUT`), with each runner's time.

How rows count:

* Batch and syntax rows run `-Q --batch -l perfbench/perfbench.el`, one
  process per run: instructions are the run's less a run doing only its setup
  (perf/font-lock's method), so a tree-sitter mode's background parsing counts
  on whatever thread it runs.  Wall is the body's own time.  Rows with one
  setup share its setup-only runs.  Each engine's result (match counts,
  positions) is compared, and a difference is noted.  The syntax rows are the
  syntax-ppss lane's rig: every operation but `font-lock-ensure` starts warm
  (the buffer's syntax state computed, one key typed), `font-lock-ensure`
  starts with the mode just turned on, so a tree-sitter mode's first parse is
  part of it; its result is the number of parsers, compared across engines.
* Lisp rows loop 1,000,000 times in a file compiled in both engines, GNU
  byte-compiled and Romacs from its compiled store, per iteration beyond an
  empty loop.  `uncompiled` rows read source in both, GNU into its
  interpreter, per whole iteration less a run that only loads the file.  elisp-benchmarks members load GNU's
  `.elc` and, when GNU has native compilation, its `.eln` (a third column),
  and Romacs's store; the compiled package is kept between runs.
* Per-key rows run in one GUI session per engine on headless labwc, as the
  parity harnesses run both.  Instructions are the whole process's over the
  keys less its idle rate, every thread but llvmpipe's (Mesa's software GL
  stands in for the GPU there); wall runs from `pre-command-hook` to the next
  idle moment, after redisplay (`perfbench-gui.el`).  Romacs takes GNU's
  completion there (fido-vertical, the styles without fzf,
  `real_init.GNU_COMPLETION_UI`), but in the `romacs-complete` row.  Code rows
  visit their file afresh each round, C in c-ts-mode in both.
* Session rows (opening files, the history log, themes) run in a session as
  each engine ships.  Wall runs from the key to the result; block is the
  longest gap between ticks of a 5 ms timer, which cannot run while the main
  VM is busy (`perfbench-session.el`).  These are read by wall and block: a
  worker VM's path does more work in total to keep the main VM free.  The log
  is upstream master's whole history with `vc-log-show-limit` 0, its commits
  and the buffer's hash compared across engines.

Romacs runs as an installation runs it.  Before anything is measured,
perfbench stages the binary beside its Lisp trees and compiles them with
`--compile-lisp-tree`, as an installer does, cached under OUT per binary and
runtime (a minute or two the first time, not part of any row).  Batch rows
set `ROMACS_COMPILED_LISP=on` and boot from a batch world prepared first.  A
workload's own file loads by name with an `.elc` beside it, as an installed
package does.  Each row's note counts the loads the store's use log confirms
and names any read as source.  `--romacs-source` measures with no store or
world.  `--neomacs BIN` adds neomacs as a third engine with its own column.

When the 1-minute load average exceeds `--max-load` (default 8) during a row,
its wall times are withheld; instructions are still reported.  The inputs are
pinned: the Emacs tree is a `--shared` clone of `$ROMACS_GNU_SOURCE` at the
oracle's commit, checked clean, its fixture files checked against SHA-256s,
and the 5,000 files have fixed contents and mtimes.  Every engine runs in
private HOME/XDG/TMPDIR trees, and no row writes a user's data.

After a whole run (it refuses one made with `--only`),
`perfbench/readme.py OUT/COMMIT/TIME/results.json` writes
the tables below, every row kept, and fills README's lead (`perfbench/lead.md`,
each `{{name}}` from the run; a name with no value stops it and leaves both
files as they were, and a wall refused for load counts as none).  It also
lists the headlines: every row where Romacs is 0.80x or better against a rival
on every metric measured, and names on standard error any headline the run
lost.  A block the run did not measure is left as it was.

## Latest results

<!-- perfbench-table-begin -->
Measured 2026-11-07 on build `19f44056` against the installed GNU Emacs 32.0.50, commit `b28750b8`, package emacs-wayland-git 32.0.50.r183254.gb28750b82.local-1, natively compiled; the inputs are pinned at `45b0f769`, on AMD Ryzen 9 5950X 16-Core Processor, 32 threads, 47 GB.  Each workload ran 3 times per engine in private home directories; wall time is the median, and wall times measured above load 8 are left out.  Romacs ran as an installation does: 1931 of the 1981 files of its Lisp compiled ahead of use into its store by `--compile-lisp-tree`, before anything was timed; batch rows booted from a batch world prepared first, which a user's first batch run builds, and a workload's own file loaded compiled.  Each row's note counts the loads the store confirms and names any it does not.  A ratio is Romacs over the other engine: below 1.00x, Romacs is faster.

| Workload | Per | GNU | Romacs | Romacs/GNU (time) | Romacs/GNU (instructions) | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| startup -Q --batch | run | 61.3 ms | 69.2 ms | 1.13x | 1.21x | process to exit; Romacs: compiled store, 0 of 0 loads compiled |
| font-lock-ensure fileio.c | run | 878 ms | 2.2 s | 2.51x | 2.44x | less its setup; Romacs: compiled store, 10 of 10 loads compiled |
| font-lock-ensure subr.el | run | 124 ms | 296 ms | 2.38x | 2.34x | less its setup; Romacs: compiled store, 0 of 0 loads compiled |
| font-lock-ensure ORG-NEWS | run | 113 ms | 420 ms | 3.73x | 3.44x | less its setup; Romacs: compiled store, 169 of 170 loads compiled, read as source: org-version.el |
| dired 5,000 files | run | 110 ms | 283 ms | 2.58x | 3.49x | less its setup; Romacs: compiled store, 3 of 3 loads compiled |
| project-find-regexp repo | run | 383 ms | 569 ms | 1.49x | 1.48x | less its setup; Romacs: compiled store, 25 of 26 loads compiled, read as source: cus-load.el |
| vc-log 2,000 commits | run | 41.5 ms | 108 ms | 2.60x | 2.24x | less its setup; Romacs: compiled store, 14 of 15 loads compiled, read as source: cus-load.el |
| occur xdisp.c | run | 17 ms | 26.3 ms | 1.54x | 1.32x | less its setup; Romacs: compiled store, 0 of 0 loads compiled |
| how-many xdisp.c | run | 4.79 ms | 778 µs | 0.16x | 0.10x | less its setup; Romacs: compiled store, 0 of 0 loads compiled |
| find-file xdisp.c | run | 124 ms | 242 ms | 1.94x | 1.91x | less its setup; Romacs: compiled store, 19 of 20 loads compiled, read as source: cus-load.el |
| describe-function find-file | run | 43.1 ms | 73.5 ms | 1.70x | 1.26x | less its setup; Romacs: compiled store, 6 of 7 loads compiled, read as source: shortdoc-doc.el |
| shell-command 20 runs | run | 25.7 ms | 30.3 ms | 1.18x | 1.24x | less its setup; Romacs: compiled store, 0 of 0 loads compiled |
| eshell 20 commands | run | 36.8 ms | 70.2 ms | 1.91x | 0.76x | less its setup; Romacs: 25 artifacts compiled on first use, before these runs; Romacs: compiled store, 33 of 33 loads compiled |
| ERT 25 files | total | 15.3 s | 19.3 s | 1.26x | 1.36x | GNU 927/929 expected, Romacs 894/929 expected; wall summed per file; Romacs: compiled store, 39 of 42 loads compiled, read as source: cus-load.el, uni-name.el, uni-old-name.el, loads not checked in 9 of 75 runs; test files read as source in both |
| first-start world build | run | - | 2.63 s | - | - | Romacs only: --loadup --world-build in a fresh store; a world build reads source, never the compiled store |
| warm launch to first paint | run | - | 260 ms | - | - | Romacs only (its --trace-latency first-paint); GNU has no paint mark; Romacs: compiled store, 0 of 0 loads compiled |
| warm launch to idle | run | 395 ms | 281 ms | 0.71x | 0.84x | spawn to the first idle after emacs-startup-hook; instructions to exit there; Romacs: compiled store, 0 of 0 loads compiled |
| key self-insert | key | 1.1 ms | 1.36 ms | 1.24x | 0.91x | 40 keys x 3; wall pre-command to idle; samples GNU 120, Romacs 120; Romacs: compiled store, 1 of 1 loads compiled |
| key C-f | key | 123 µs | 1.11 ms | 9.00x | 1.26x | 40 keys x 3; wall pre-command to idle; samples GNU 120, Romacs 120; Romacs: compiled store, 1 of 1 loads compiled |
| key C-n | key | 2.43 ms | 1.45 ms | 0.59x | 0.77x | 40 keys x 3; wall pre-command to idle; samples GNU 120, Romacs 120; Romacs: compiled store, 1 of 1 loads compiled |
| C-x C-f 5,000 files, per key | key | 69.5 ms | 51.8 ms | 0.75x | 0.67x | fido-vertical, keys of file-1; candidates GNU 1000, Romacs 1000; Romacs: compiled store, 1 of 1 loads compiled |
| C-x C-f 5,000 files, per key, romacs-complete | key | - | 2.83 ms | - | - | Romacs only: its default completion (the layer, fzf), keys of file-1; candidates 2084, minibuffer 'file-1'; Romacs: compiled store, 1 of 1 loads compiled |
| micro named call | iteration | 9.14 ns | 5.07 ns | 0.55x | 0.59x | (m-id i); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro closure call | iteration | 8.53 ns | 18.7 ns | 2.19x | 1.85x | (funcall f0 i); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro mapcar 1 | iteration | 95.4 ns | 128 ns | 1.34x | 1.45x | (mapcar f0 (cdr (cdr (cdr (cdr (cdr (cdr (cdr (cdr (cdr l)))))))))); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro mapcar 10 | iteration | 730 ns | 632 ns | 0.87x | 0.88x | (mapcar f0 l); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro dolist + funcall 10 | iteration | 168 ns | 261 ns | 1.56x | 1.61x | (dolist (x l) (funcall f0 x)); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| uncompiled arithmetic | iteration | 149 ns | 18.4 ns | 0.12x | 0.15x | (setq s (+ s (% i 97))); GNU interpreted; Romacs: compiled store, 0 of 1 loads compiled, read as source: b2d0c8442cb8.el |
| uncompiled named call | iteration | 257 ns | 16.2 ns | 0.06x | 0.13x | (m-id i); GNU interpreted; Romacs: compiled store, 0 of 1 loads compiled, read as source: c38ab8dd7f42.el |
| uncompiled dolist 10 | iteration | 2.87 µs | 165 ns | 0.06x | 0.06x | (dolist (x l) (setq s (+ s x))); GNU interpreted; Romacs: compiled store, 0 of 1 loads compiled, read as source: daba0119891f.el |
<!-- perfbench-table-end -->

### Sessions

<!-- perfbench-session-begin -->
Real commands and keys in a GUI session.  Wall runs from the key to the result.  Block is the longest gap between ticks of a 5 ms timer, which cannot run while the main VM is busy: how long the session stopped answering.  Read these rows by wall and block: work moved to a worker VM adds instructions and removes block.

Measured 2026-11-07 on build `19f44056` against the installed GNU Emacs 32.0.50, commit `b28750b8`, package emacs-wayland-git 32.0.50.r183254.gb28750b82.local-1, natively compiled; the inputs are pinned at `45b0f769`, on AMD Ryzen 9 5950X 16-Core Processor, 32 threads, 47 GB.  Each row ran 3 times per engine in private home directories; wall time is the median, and wall times measured above load 8 are left out.  Romacs ran as an installation does: 1931 of the 1981 files of its Lisp compiled ahead of use into its store by `--compile-lisp-tree`, before anything was timed; batch rows booted from a batch world prepared first, which a user's first batch run builds, and a workload's own file loaded compiled.  Each row's note counts the loads the store confirms and names any it does not.  A ratio is Romacs over the other engine: below 1.00x, Romacs is faster.

| Row | Per | GNU | Romacs | GNU block | Romacs block | Romacs/GNU (time) | Romacs/GNU (block) | Romacs/GNU (instructions) | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| dired 5,000 files | run | 127 ms | 292 ms | 124 ms | 292 ms | 2.30x | 2.35x | 3.04x | CPU cores GNU 0.4, Romacs 0.5; Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| occur xdisp.c | run | 1.55 s | 1.94 s | 1.55 s | 1.94 s | 1.25x | 1.25x | 1.66x | CPU cores GNU 0.8, Romacs 1.9; Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| project-find-regexp | run | 434 ms | 593 ms | 432 ms | 58.9 ms | 1.37x | 0.14x | 0.31x | CPU cores GNU 0.6, Romacs 0.7; Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| project-find-file | run | 37.2 ms | 71.7 ms | 36.7 ms | 16.2 ms | 1.93x | 0.44x | 1.02x | CPU cores GNU 0.3, Romacs 0.4; Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| vc-log 2,000 commits | run | 48.6 ms | 41.2 ms | 19.5 ms | 11.3 ms | 0.85x | 0.58x | 1.11x | CPU cores GNU 0.3, Romacs 0.4; Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| vc-diff xdisp.c | run | 294 ms | 289 ms | 292 ms | 288 ms | 0.98x | 0.99x | 1.55x | CPU cores GNU 0.6, Romacs 1.5; Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| vc-dir | run | 46 ms | 52.2 ms | 13.8 ms | 18.3 ms | 1.13x | 1.33x | 4.38x | CPU cores GNU 0.4, Romacs 0.4; Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| M-x shell, one command | run | 2.81 ms | 29 ms | 2.8 ms | 28.9 ms | 10.31x | 10.35x | - | CPU cores GNU 0.2, Romacs 0.4; Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| org-agenda, a month | run | 3.05 s | 11.7 s | 3.05 s | 11.7 s | 3.83x | 3.83x | 4.09x | CPU cores GNU 0.9, Romacs 1.0; Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| kill-line, per key | key | 1.76 ms | 1.58 ms | 32.4 ms | 6.67 ms | 0.90x | 0.21x | 13.85x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| yank, per key | key | 1.64 ms | 1.7 ms | 29.7 ms | 6.75 ms | 1.03x | 0.23x | 3.29x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| undo, per key | key | 1.1 ms | 1.34 ms | 31 ms | 6.59 ms | 1.23x | 0.21x | 70.88x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| isearch xdisp.c, per key | key | 2.94 ms | 3.95 ms | 28.8 ms | 18.2 ms | 1.34x | 0.63x | 1.64x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| Org self-insert, per key | key | 1.23 ms | 3.2 ms | 35.6 ms | 77.7 ms | 2.60x | 2.18x | 18.56x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| Org TAB, per key | key | 2.43 ms | 42.8 ms | 36.6 ms | 331 ms | 17.60x | 9.06x | 49.24x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| find-file xdisp.c | run | 154 ms | 228 ms | 149 ms | 228 ms | 1.48x | 1.53x | 1.89x | first shown GNU 149 ms, Romacs 228 ms; CPU cores GNU 0.4, Romacs 0.4; mode GNU c-mode, Romacs c-ts-mode; Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| find-file generated header | run | 6.85 s | 1.05 s | 5.45 s | 1.05 s | 0.15x | 0.19x | 0.26x | first shown GNU 5.45 s, Romacs 1.05 s; CPU cores GNU 0.9, Romacs 0.7; mode GNU c-mode, Romacs c-ts-mode; Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| find-file xdisp.c, tree-sitter in both | run | 433 ms | 227 ms | 407 ms | 227 ms | 0.53x | 0.56x | 0.54x | first shown GNU 23.6 ms, Romacs 227 ms; CPU cores GNU 0.6, Romacs 0.4; mode GNU c-ts-mode, Romacs c-ts-mode; Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| find-file generated header, tree-sitter in both | run | 1.94 s | 1.07 s | 1.91 s | 1.07 s | 0.55x | 0.56x | 0.50x | first shown GNU 27.9 ms, Romacs 1.07 s; CPU cores GNU 0.8, Romacs 0.7; mode GNU c-ts-mode, Romacs c-ts-mode; Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| vc-print-root-log whole history | run | 18 s | 2.82 s | 57.5 ms | 35.7 ms | 0.16x | 0.62x | 0.25x | first shown GNU 9.81 ms, Romacs 9.55 ms; median key GNU 4.78 ms, Romacs 4.82 ms; CPU cores GNU 1.1, Romacs 2.4; Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| theme switch, first visit | run | 18.5 ms | 13.6 ms | 17.5 ms | 13.6 ms | 0.74x | 0.78x | 6.83x | CPU cores GNU 0.3, Romacs 0.1; Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more; each engine on its own shipped themes |
| theme switch, cycled | run | 23.7 ms | 4.5 ms | 22.4 ms | 4.48 ms | 0.19x | 0.20x | 0.62x | CPU cores GNU 0.3, Romacs 0.1; Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more; each engine on its own shipped themes |
<!-- perfbench-session-end -->

### elisp-benchmarks

<!-- perfbench-elb-begin -->
This is GNU's own benchmark suite, GNU ELPA's elisp-benchmarks (the one GNU uses to evaluate native compilation), not ours, and it measures pure Lisp compute, not editor responsiveness.

Measured 2026-11-07 on build `19f44056` against the installed GNU Emacs 32.0.50, commit `b28750b8`, package emacs-wayland-git 32.0.50.r183254.gb28750b82.local-1, natively compiled; the inputs are pinned at `45b0f769`, on AMD Ryzen 9 5950X 16-Core Processor, 32 threads, 47 GB.  Each member ran 3 times per engine in private home directories; wall time is the median, and wall times measured above load 8 are left out.  GNU loads the member files byte-compiled, with JIT native compilation off, and natively compiled at the suite's own speed and safety; its own Lisp libraries are natively compiled in both columns, as this GNU is built with native compilation.  Romacs ran as an installation does: 1931 of the 1981 files of its Lisp compiled ahead of use into its store by `--compile-lisp-tree`, before anything was timed; batch rows booted from a batch world prepared first, which a user's first batch run builds, and a workload's own file loaded compiled.  Each row's note counts the loads the store confirms and names any it does not.  A member an engine does not define, or whose run reported an error, has no figure there.  A ratio is Romacs over the other engine: below 1.00x, Romacs is faster.  The geometric mean compares Romacs with GNU `.elc` alone.  Every ratio also has a median over the members that have it, and the median is the summary to read against native GNU: native compilation reduces some members to almost nothing, and one such ratio would decide a mean.

| Member | GNU .elc | GNU native | Romacs | Romacs/GNU .elc (time) | Romacs/GNU native (time) | Romacs/GNU .elc (instructions) | Romacs/GNU native (instructions) | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| bubble | 2.25 s | 658 ms | 2.01 s | 0.89x | 3.05x | 0.92x | 3.09x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| bubble-no-cons | 6.04 s | 633 ms | 4.08 s | 0.68x | 6.45x | 0.66x | 3.45x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| bytecomp | 849 ms | 847 ms | - | - | - | - | - | Romacs: reported "Error: Symbol’s function definition is void: read-positioning-symbols"; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| dhrystone | 2.24 s | 1.51 s | 5.75 s | 2.56x | 3.79x | 2.85x | 3.99x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| eieio | 777 ms | 779 ms | 2.6 s | 3.34x | 3.33x | 2.96x | 2.97x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| fibn | 1.52 s | 5.01 µs | 864 ms | 0.57x | 172514.43x | 0.47x | 105199.68x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| fibn-named-let | 1.81 s | 849 ms | 1.5 s | 0.83x | 1.76x | 0.55x | 0.77x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| fibn-rec | 1.74 s | 14.8 µs | 2.4 s | 1.37x | 162073.77x | 1.48x | 898311.08x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| fibn-tc | 1.68 s | 2.31 ms | 2.07 s | 1.23x | 896.46x | 1.39x | 767.59x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| flet | 4.04 s | 965 ms | 4.79 s | 1.18x | 4.97x | 1.23x | 5.19x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| font-lock | 420 ms | 416 ms | 969 ms | 2.31x | 2.33x | 2.07x | 2.08x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| inclist | 7.59 s | 749 ms | 5.98 s | 0.79x | 7.99x | 0.83x | 9.06x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| inclist-type-hints | 7.38 s | 590 ms | 6.35 s | 0.86x | 10.77x | 0.83x | 18.12x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| listlen-tc | 1.55 s | 84.5 ms | 7.77 s | 5.01x | 91.93x | 1.76x | 33.75x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| map-closure | 3.23 s | 3.14 s | 6.67 s | 2.06x | 2.13x | 1.84x | 1.84x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| nbody | 2.19 s | 1.28 s | 2.99 s | 1.36x | 2.34x | 1.33x | 1.80x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| pack-unpack | 224 ms | 222 ms | 726 ms | 3.24x | 3.27x | 3.45x | 3.45x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| pack-unpack-old | 371 ms | 372 ms | 2.09 s | 5.63x | 5.61x | 4.33x | 4.33x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| pcase | 3.79 s | 1.71 s | 5.34 s | 1.41x | 3.13x | 1.67x | 3.18x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| pidigits | 3.25 s | 3.13 s | 16.1 s | 4.94x | 5.14x | 3.74x | 3.75x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| scroll | 392 ms | 396 ms | - | - | - | - | - | Romacs: no elb-scroll-entry after loading the suite |
| smie | 906 ms | 920 ms | 2.88 s | 3.18x | 3.13x | 2.81x | 2.82x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| geometric mean | 1.83 s | - | 3.13 s | 1.71x | - | 1.54x | - | against GNU .elc, over the 20 of 22 members both ran; left out: bytecomp, scroll |
| median | - | - | - | 1.39x | 4.38x | 1.58x | 3.60x | median of the per-member ratios, members with instructions figures: GNU .elc 20, GNU native 20 |
<!-- perfbench-elb-end -->

### Headlines

<!-- perfbench-headlines-begin -->
Every row of this run where Romacs beats a rival by at least 20%: 0.80x or better on every metric the run measured.  Build `19f44056`.  The tables keep every row, losses included.

| Row | Against | Instructions | Time | Block | Build | Rerun |
| --- | --- | --- | --- | --- | --- | --- |
| how-many xdisp.c | GNU | 0.10x | 0.16x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'how-many xdisp.c'` |
| elb bubble-no-cons | GNU | 0.66x | 0.68x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'elb'` |
| elb fibn | GNU | 0.47x | 0.57x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'elb'` |
| key C-n | GNU | 0.77x | 0.59x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'keys'` |
| C-x C-f 5,000 files, per key | GNU | 0.67x | 0.75x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'keys'` |
| micro named call | GNU | 0.59x | 0.55x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'micro'` |
| uncompiled arithmetic | GNU | 0.15x | 0.12x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'micro'` |
| uncompiled named call | GNU | 0.13x | 0.06x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'micro'` |
| uncompiled dolist 10 | GNU | 0.06x | 0.06x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'micro'` |
| session find-file generated header | GNU | 0.26x | 0.15x | 0.19x | `19f44056` | `perfbench.py --romacs BINARY --only 'session'` |
| session find-file xdisp.c, tree-sitter in both | GNU | 0.54x | 0.53x | 0.56x | `19f44056` | `perfbench.py --romacs BINARY --only 'session'` |
| session find-file generated header, tree-sitter in both | GNU | 0.50x | 0.55x | 0.56x | `19f44056` | `perfbench.py --romacs BINARY --only 'session'` |
| session vc-print-root-log whole history | GNU | 0.25x | 0.16x | 0.62x | `19f44056` | `perfbench.py --romacs BINARY --only 'session'` |
| session theme switch, first visit | GNU | 6.83x | 0.74x | 0.78x | `19f44056` | `perfbench.py --romacs BINARY --only 'session'` |
| session theme switch, cycled | GNU | 0.62x | 0.19x | 0.20x | `19f44056` | `perfbench.py --romacs BINARY --only 'session'` |
<!-- perfbench-headlines-end -->

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

## What one redisplay costs (`display_stages_live.py`)

Redisplay's cost by scenario, in instructions, held to bars. Each scenario
of `display_stages.el` (synthetic buffers: plain text, text with invisible
and `display` parts, an agent chat) runs in a fresh `-Q --loadup` session
in a private headless labwc: a buffer shown one way, then 30 steps, each a
timer that makes the scenario's change followed by the loop's own
redisplay. The session reads `romacs-instructions` and
`romacs--display-stats` at each step's start, all before it writes
anything, so the report has, per step and as medians, the instructions and
conses spent inside redisplay beside the whole step's instructions, the
windows rebuilt, the bands expanded, the layouts asked of the layout host
and the frames submitted. `display_stages_bars.json` holds each scenario's
figures: instructions and conses under a ceiling, the counts exactly, so a
step that no longer does its change fails too. So does a scenario more
than a tenth of whose steps paint when they change nothing, or do not when
they do. Work that removes cost lowers the bars in the same change:
`--write-bars FILE` writes the ones a run would pass. `--record` adds a
call graph of each scenario's steps (a frame-pointer build).

```sh
python3 -B crates/romacs-vm-gui/harness/display_stages_live.py --binary BIN --output DIR \
  [--only NAME ...] [--bars FILE] [--write-bars FILE] [--record]
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
