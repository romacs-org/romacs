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
with `-l` (Romacs as `--loadup -Q -l`).  Both use only the stock lisp and lisp/romacs, on a
generic project at `~/src/tally` (`bench/fixture.py`: Emacs Lisp, C, Rust,
Markdown, Org and a Makefile, with a five-commit history whose authors and
dates are fixed, so the hashes match everywhere).  `init` is a real-init
profile, described by a JSON profile config kept outside the repository
(`--profile-config FILE` or `ROMACS_BENCH_PROFILE`): the init's blocks, the
package sources it loads, the project to clone, an org file, and a gnosis
database copy; `real_init.py`'s docstring
lists the keys.  Both
profiles give GNU the settings Romacs's `--loadup` makes by design, as the
parity harnesses do, and Romacs GNU's completion: fido-vertical, not its
own completion layer (`romacs-complete-mode`), which takes the root
window's last line, and the completion styles without fzf.  Each engine serves
`lisp/agents/romacs-mcp.el` with the session tools from
`lisp/agents/romacs-mcp-session.el`.  The same Lisp runs in both engines, so both
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
refused.

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
Measured 2026-11-14 on build `c4f4cebe` against GNU Emacs 32.0.50 pinned at commit `45b0f769`, natively compiled, the inputs pinned there too, on AMD Ryzen 9 5950X 16-Core Processor, 32 threads, 47 GB.  Each workload ran 3 times per engine in private home directories; instructions (`instructions:u`, every thread) are the main figure and wall time the median beside them; wall times measured above load 8 are left out.  The whole run took 13 min 3 s.  Rerun: `python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py --romacs BINARY`, then `python3 -B crates/romacs-vm-gui/harness/perfbench/readme.py OUT/COMMIT/TIME/results.json`; one area or row: `--only AREA` or `--only 'ROW'`.  Romacs ran as an installation does: 1956 of the 2005 files of its Lisp compiled ahead of use into its store by `--compile-lisp-tree`, before anything was timed; batch rows booted from a batch world prepared first, which a user's first batch run builds, and a workload's own file loaded compiled.  Each row's note counts the loads the store confirms and names any it does not.  A ratio is Romacs over the other engine: below 1.00x, Romacs is faster.

| Workload | Per | GNU instructions | Romacs instructions | Romacs/GNU (instructions) | GNU time | Romacs time | Romacs/GNU (time) | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| startup -Q --batch | run | 259M | 457M | 1.77x | - | - | - | wall refused: load 8.4 > 8.0; process to exit; Romacs: compiled store, 0 of 0 loads compiled |
| dired 5000 files | run | 800M | 1.6G | 2.00x | 110 ms | 210 ms | 1.91x | less its setup; Romacs: compiled store, 3 of 3 loads compiled |
| occur xdisp.c | run | 183M | 248M | 1.36x | 20.6 ms | 32.1 ms | 1.56x | less its setup; Romacs: compiled store, 0 of 0 loads compiled |
| project-find-regexp repo | run | 2.63G | 3.59G | 1.36x | 404 ms | 579 ms | 1.43x | less its setup; Romacs: compiled store, 25 of 26 loads compiled, read as source: cus-load.el |
| describe-function find-file | run | 590M | 776M | 1.32x | 44.6 ms | 68.7 ms | 1.54x | less its setup; Romacs: compiled store, 6 of 7 loads compiled, read as source: shortdoc-doc.el |
| shell-command 20 runs | run | 42.4M | 57.4M | 1.36x | 31.3 ms | 36.8 ms | 1.18x | less its setup; Romacs: compiled store, 0 of 0 loads compiled |
| font-lock-ensure ORG-NEWS | run | 1.39G | 4.38G | 3.14x | 115 ms | 437 ms | 3.80x | less its setup; Romacs: compiled store, 171 of 172 loads compiled, read as source: org-version.el |
| syntax-ppss subr.el emacs-lisp-mode | run | 1.69G | 2.56G | 1.52x | 97.4 ms | 175 ms | 1.80x | 10 edits near the start, each then (syntax-ppss (point-max)); less its setup; Romacs: compiled store, 1 of 1 loads compiled |
| scan-lists subr.el emacs-lisp-mode | run | 186M | 249M | 1.34x | 6.74 ms | 20.4 ms | 3.02x | 500 times out of the top-level form around the middle, both ways; less its setup; Romacs: compiled store, 1 of 1 loads compiled |
| forward-comment subr.el emacs-lisp-mode | run | 611M | 682M | 1.12x | 28.8 ms | 53.6 ms | 1.86x | 1000 times over the file's leading comments and back; less its setup; Romacs: compiled store, 1 of 1 loads compiled |
| key in the middle subr.el emacs-lisp-mode | run | 54.1M | 21.9M | 0.41x | 565 µs | 3.57 ms | 6.32x | 20 self-inserted keys, each line refontified and its paren shown; less its setup; Romacs: compiled store, 1 of 1 loads compiled |
| font-lock-ensure subr.el emacs-lisp-mode | run | 1.78G | 3.27G | 1.83x | 128 ms | 322 ms | 2.52x | the whole buffer, the mode just turned on; less its setup; Romacs: compiled store, 1 of 1 loads compiled |
| syntax-ppss fileio.c c-mode | run | 378M | 130M | 0.34x | 21 ms | 12.6 ms | 0.60x | 10 edits near the start, each then (syntax-ppss (point-max)); less its setup; Romacs: compiled store, 11 of 11 loads compiled |
| scan-lists fileio.c c-mode | run | 219M | 401M | 1.83x | 10.8 ms | 33.4 ms | 3.09x | 500 times out of the top-level form around the middle, both ways; less its setup; Romacs: compiled store, 11 of 11 loads compiled |
| forward-comment fileio.c c-mode | run | 363M | 478M | 1.32x | 17.3 ms | 38.1 ms | 2.21x | 1000 times over the file's leading comments and back; less its setup; Romacs: compiled store, 11 of 11 loads compiled |
| key in the middle fileio.c c-mode | run | 239M | 497M | 2.08x | 21.4 ms | 59.6 ms | 2.79x | 20 self-inserted keys, each line refontified and its paren shown; less its setup; Romacs: compiled store, 11 of 11 loads compiled |
| font-lock-ensure fileio.c c-mode | run | 10.4G | 19.2G | 1.85x | 883 ms | 2.19 s | 2.48x | the whole buffer, the mode just turned on; less its setup; Romacs: compiled store, 11 of 11 loads compiled |
| syntax-ppss xdisp.c c-mode | run | 2.08G | 454M | 0.22x | 113 ms | 38 ms | 0.34x | 10 edits near the start, each then (syntax-ppss (point-max)); less its setup; Romacs: compiled store, 11 of 11 loads compiled |
| scan-lists xdisp.c c-mode | run | 1.9G | 3.51G | 1.85x | 95.7 ms | 273 ms | 2.86x | 500 times out of the top-level form around the middle, both ways; less its setup; Romacs: compiled store, 11 of 11 loads compiled |
| forward-comment xdisp.c c-mode | run | 11.9G | 14.9G | 1.25x | 549 ms | 1.18 s | 2.15x | 1000 times over the file's leading comments and back; less its setup; Romacs: compiled store, 11 of 11 loads compiled |
| key in the middle xdisp.c c-mode | run | 332M | 786M | 2.37x | 27 ms | 76.1 ms | 2.82x | 20 self-inserted keys, each line refontified and its paren shown; less its setup; Romacs: compiled store, 11 of 11 loads compiled |
| syntax-ppss fileio.c c-ts-mode | run | 361M | 29.8M | 0.08x | 19.2 ms | 2.45 ms | 0.13x | 10 edits near the start, each then (syntax-ppss (point-max)); less its setup; Romacs: compiled store, 4 of 4 loads compiled |
| scan-lists fileio.c c-ts-mode | run | 211M | 354M | 1.68x | 10.7 ms | 28.8 ms | 2.69x | 500 times out of the top-level form around the middle, both ways; less its setup; Romacs: compiled store, 4 of 4 loads compiled |
| forward-comment fileio.c c-ts-mode | run | 110M | 273M | 2.48x | 12.7 ms | 33.5 ms | 2.63x | 1000 times over the file's leading comments and back; less its setup; Romacs: compiled store, 4 of 4 loads compiled |
| key in the middle fileio.c c-ts-mode | run | 5.58G | 1.99G | 0.36x | 409 ms | 166 ms | 0.40x | 20 self-inserted keys, each line refontified and its paren shown; less its setup; Romacs: compiled store, 4 of 4 loads compiled |
| font-lock-ensure fileio.c c-ts-mode | run | 2.24G | 1.56G | 0.69x | 164 ms | 116 ms | 0.71x | the whole buffer, the mode just turned on; less its setup; Romacs: compiled store, 4 of 4 loads compiled |
| syntax-ppss xdisp.c c-ts-mode | run | 2.01G | 40M | 0.02x | 109 ms | 3.38 ms | 0.03x | 10 edits near the start, each then (syntax-ppss (point-max)); less its setup; Romacs: compiled store, 4 of 4 loads compiled |
| scan-lists xdisp.c c-ts-mode | run | 1.93G | 3.21G | 1.66x | 101 ms | 251 ms | 2.48x | 500 times out of the top-level form around the middle, both ways; less its setup; Romacs: compiled store, 4 of 4 loads compiled |
| forward-comment xdisp.c c-ts-mode | run | 138M | 398M | 2.88x | 15.8 ms | 52.6 ms | 3.33x | 1000 times over the file's leading comments and back; less its setup; Romacs: compiled store, 4 of 4 loads compiled |
| key in the middle xdisp.c c-ts-mode | run | 3.62G | 1.39G | 0.38x | 290 ms | 118 ms | 0.41x | 20 self-inserted keys, each line refontified and its paren shown; less its setup; Romacs: compiled store, 4 of 4 loads compiled |
| font-lock-ensure xdisp.c c-ts-mode | run | 11.4G | 8.26G | 0.73x | 844 ms | 637 ms | 0.75x | the whole buffer, the mode just turned on; less its setup; Romacs: compiled store, 4 of 4 loads compiled |
| micro named call | iteration | 216 | 115 | 0.53x | 10.1 ns | 5.4 ns | 0.53x | (m-id i); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro closure call | iteration | 213 | 427 | 2.01x | 9.75 ns | 21.7 ns | 2.22x | (funcall f0 i); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro mapcar 10 | iteration | 8.23k | 9.78k | 1.19x | 657 ns | 583 ns | 0.89x | (mapcar f0 l); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro dolist + funcall 10 | iteration | 3.1k | 5.03k | 1.62x | 190 ns | 290 ns | 1.53x | (dolist (x l) (funcall f0 x)); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| prim gethash | iteration | 424 | 1.1k | 2.59x | 24.5 ns | 62.1 ns | 2.54x | (gethash (% i 1000) m-h); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| prim puthash | iteration | 327 | 1.19k | 3.63x | 18.4 ns | 66.9 ns | 3.63x | (puthash (% i 1000) i m-h); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| prim assq | iteration | 292 | 573 | 1.96x | 10.8 ns | 41.3 ns | 3.84x | (assq 'j m-al); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| prim plist-get | iteration | 529 | 557 | 1.05x | 25.4 ns | 71.8 ns | 2.83x | (plist-get m-pl :j); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| prim format | iteration | 3.54k | 4.55k | 1.28x | 283 ns | 378 ns | 1.33x | (format "%d-%s" i "x"); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| prim concat | iteration | 1.85k | 2.17k | 1.17x | 165 ns | 186 ns | 1.13x | (concat (car m-s) "_row"); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| prim string-match | iteration | 2.37k | 4.17k | 1.76x | 157 ns | 352 ns | 2.24x | (string-match "_\\([a-z]+\\)_" "glyph_row_struct"); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| prim intern | iteration | 359 | 1.9k | 5.30x | 21.6 ns | 113 ns | 5.21x | (intern "glyph_row"); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| prim insert + delete-char | iteration | 9.41k | 20.2k | 2.14x | 702 ns | 1.78 µs | 2.53x | (let ((buffer-undo-list t)) (insert "x") (delete-char -1)); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| uncompiled arithmetic | iteration | 3.35k | 243 | 0.07x | 147 ns | 16.8 ns | 0.11x | (setq s (+ s (% i 97))); GNU interpreted; less a run that only loads it; Romacs: compiled store, 0 of 1 loads compiled, read as source: 7c68928cf438.el |
| uncompiled named call | iteration | 3.96k | 253 | 0.06x | 228 ns | 13.4 ns | 0.06x | (m-id i); GNU interpreted; less a run that only loads it; Romacs: compiled store, 0 of 1 loads compiled, read as source: be2225980221.el |
| uncompiled dolist 10 | iteration | 44.3k | 2.3k | 0.05x | 2.73 µs | 158 ns | 0.06x | (dolist (x l) (setq s (+ s x))); GNU interpreted; less a run that only loads it; Romacs: compiled store, 0 of 1 loads compiled, read as source: 4b1ecf859fad.el |
| launch to first paint | run | - | - | - | - | 291 ms | - | Romacs only (its --trace-latency first-paint); GNU has no paint mark; Romacs: compiled store, 2 of 2 loads compiled |
| launch to idle | run | 3.26G | 3.17G | 0.97x | 421 ms | 326 ms | 0.77x | spawn to the first idle after emacs-startup-hook; instructions to exit there; Romacs: compiled store, 2 of 2 loads compiled |
| key self-insert text | key | 32.3M | 35.6M | 1.10x | 1.09 ms | 1.02 ms | 0.94x | 20 keys a round, latencies GNU 60, Romacs 60; Romacs: compiled store, 22 of 23 loads compiled, read as source: cus-load.el |
| key C-f text | key | 20.4M | 34.2M | 1.68x | 85.9 µs | 768 µs | 8.93x | 20 keys a round, latencies GNU 60, Romacs 60; Romacs: compiled store, 22 of 23 loads compiled, read as source: cus-load.el |
| key C-n text | key | 22M | 37.4M | 1.70x | 273 µs | 1.14 ms | 4.19x | 20 keys a round, latencies GNU 60, Romacs 60; Romacs: compiled store, 22 of 23 loads compiled, read as source: cus-load.el |
| key self-insert xdisp.c c-ts-mode | key | 208M | 119M | 0.57x | 15.2 ms | 1.15 ms | 0.08x | from the middle, each round in the file visited afresh; 20 keys a round, latencies GNU 60, Romacs 60; Romacs: compiled store, 22 of 23 loads compiled, read as source: cus-load.el |
| key self-insert subr.el emacs-lisp-mode | key | 26M | 31.9M | 1.23x | 506 µs | 1.81 ms | 3.57x | from the middle, each round in the file visited afresh; 20 keys a round, latencies GNU 60, Romacs 60; Romacs: compiled store, 22 of 23 loads compiled, read as source: cus-load.el |
| scroll C-v xdisp.c c-ts-mode | key | 48.3M | 52.8M | 1.09x | 1.94 ms | 3.39 ms | 1.75x | from the top, each round in the file visited afresh; 20 keys a round, latencies GNU 60, Romacs 60; Romacs: compiled store, 22 of 23 loads compiled, read as source: cus-load.el |
| scroll C-v subr.el emacs-lisp-mode | key | 51.4M | 62.2M | 1.21x | 2.33 ms | 3.82 ms | 1.64x | from the top, each round in the file visited afresh; 20 keys a round, latencies GNU 60, Romacs 60; Romacs: compiled store, 22 of 23 loads compiled, read as source: cus-load.el |
| C-x C-f 5000 files fido | key | 700M | 629M | 0.90x | 73.1 ms | 70.3 ms | 0.96x | fido-vertical; the keys of file-1, latencies GNU 18, Romacs 18; candidates GNU 1000, Romacs 1000; Romacs: compiled store, 22 of 23 loads compiled, read as source: cus-load.el |
| M-x fido | key | 121M | 111M | 0.91x | 15.9 ms | 8.35 ms | 0.52x | fido-vertical; the keys of find-fi, latencies GNU 21, Romacs 21; candidates GNU 62, Romacs 63; Romacs: compiled store, 22 of 23 loads compiled, read as source: cus-load.el |
| C-x C-f 5000 files romacs-complete | key | - | 50.8M | - | - | 3.63 ms | - | Romacs only: its default completion (the layer, fzf); the keys of file-1, latencies Romacs 18; candidates Romacs 2084; Romacs: compiled store, 22 of 23 loads compiled, read as source: cus-load.el |
<!-- perfbench-table-end -->

### Sessions

<!-- perfbench-session-begin -->
Real commands and keys in a GUI session.  Wall runs from the key to the result.  Block is the longest gap between ticks of a 5 ms timer, which cannot run while the main VM is busy: how long the session stopped answering.  Read these rows by wall and block: work moved to a worker VM adds instructions and removes block.

Measured 2026-11-14 on build `c4f4cebe` against GNU Emacs 32.0.50 pinned at commit `45b0f769`, natively compiled, the inputs pinned there too, on AMD Ryzen 9 5950X 16-Core Processor, 32 threads, 47 GB.  Each row ran 3 times per engine in private home directories; instructions (`instructions:u`, every thread) are the main figure and wall time the median beside them; wall times measured above load 8 are left out.  The whole run took 13 min 3 s.  Rerun: `python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py --romacs BINARY`, then `python3 -B crates/romacs-vm-gui/harness/perfbench/readme.py OUT/COMMIT/TIME/results.json`; one area or row: `--only AREA` or `--only 'ROW'`.  Romacs ran as an installation does: 1956 of the 2005 files of its Lisp compiled ahead of use into its store by `--compile-lisp-tree`, before anything was timed; batch rows booted from a batch world prepared first, which a user's first batch run builds, and a workload's own file loaded compiled.  Each row's note counts the loads the store confirms and names any it does not.  A ratio is Romacs over the other engine: below 1.00x, Romacs is faster.

| Row | Per | GNU | Romacs | GNU block | Romacs block | Romacs/GNU (time) | Romacs/GNU (block) | Romacs/GNU (instructions) | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| open xdisp.c c-ts-mode | run | 436 ms | 218 ms | 407 ms | 43.7 ms | 0.50x | 0.11x | 0.57x | first shown GNU 22 ms, Romacs 31.3 ms; CPU cores GNU 0.6, Romacs 0.5; mode GNU c-ts-mode, Romacs c-ts-mode; Romacs: compiled store, 38 of 45 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| open 15 MB header c-ts-mode | run | 1.96 s | 1.07 s | 1.88 s | 129 ms | 0.55x | 0.07x | 0.50x | first shown GNU 42.8 ms, Romacs 129 ms; CPU cores GNU 0.8, Romacs 0.8; mode GNU c-ts-mode, Romacs c-ts-mode; Romacs: compiled store, 38 of 45 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| vc-print-root-log whole history | run | 14.7 s | 3 s | 46.5 ms | 53.4 ms | 0.20x | 1.15x | 0.22x | first shown GNU 53 ms, Romacs 53.4 ms; median key GNU 4.89 ms, Romacs 4.86 ms; CPU cores GNU 1.2, Romacs 2.5; Romacs: compiled store, 38 of 45 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| theme switch first visit | run | 30.2 ms | 18.7 ms | 29.1 ms | 18.6 ms | 0.62x | 0.64x | 6.56x | CPU cores GNU 0.2, Romacs 0.2; Romacs: compiled store, 38 of 45 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more; each engine on its own shipped themes |
| theme switch cycled | run | 5.08 ms | 5.7 ms | 4.51 ms | 5.67 ms | 1.12x | 1.26x | - | CPU cores GNU 0.2, Romacs 0.1; Romacs: compiled store, 38 of 45 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more; each engine on its own shipped themes |
<!-- perfbench-session-end -->

### elisp-benchmarks

<!-- perfbench-elb-begin -->
This is GNU's own benchmark suite, GNU ELPA's elisp-benchmarks (the one GNU uses to evaluate native compilation), not ours, and it measures pure Lisp compute, not editor responsiveness.

Measured 2026-11-14 on build `c4f4cebe` against GNU Emacs 32.0.50 pinned at commit `45b0f769`, natively compiled, the inputs pinned there too, on AMD Ryzen 9 5950X 16-Core Processor, 32 threads, 47 GB.  Each member ran 3 times per engine in private home directories; instructions (`instructions:u`, every thread) are the main figure and wall time the median beside them; wall times measured above load 8 are left out.  The whole run took 13 min 3 s.  Rerun: `python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py --romacs BINARY`, then `python3 -B crates/romacs-vm-gui/harness/perfbench/readme.py OUT/COMMIT/TIME/results.json`; one area or row: `--only AREA` or `--only 'ROW'`.  GNU loads the member files byte-compiled, with JIT native compilation off, and natively compiled at the suite's own speed and safety; its own Lisp libraries are natively compiled in both columns, as this GNU is built with native compilation.  Romacs ran as an installation does: 1956 of the 2005 files of its Lisp compiled ahead of use into its store by `--compile-lisp-tree`, before anything was timed; batch rows booted from a batch world prepared first, which a user's first batch run builds, and a workload's own file loaded compiled.  Each row's note counts the loads the store confirms and names any it does not.  A member an engine does not define, or whose run reported an error, has no figure there.  A ratio is Romacs over the other engine: below 1.00x, Romacs is faster.

| Member | GNU .elc | GNU native | Romacs | Romacs/GNU .elc (time) | Romacs/GNU native (time) | Romacs/GNU .elc (instructions) | Romacs/GNU native (instructions) | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| eieio | 802 ms | 779 ms | 2.98 s | 3.72x | 3.83x | 3.59x | 3.60x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| pack-unpack | 211 ms | 212 ms | 880 ms | 4.17x | 4.16x | 4.14x | 4.30x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| smie | 917 ms | 896 ms | 3.18 s | 3.47x | 3.55x | 2.28x | 2.29x | Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
<!-- perfbench-elb-end -->

### Headlines

<!-- perfbench-headlines-begin -->
Every row of this run where Romacs beats a rival by at least 20%: 0.80x or better on every metric the run measured.  Build `c4f4cebe`.  The tables keep every row, losses included.

| Row | Against | Instructions | Time | Block | Build | Rerun |
| --- | --- | --- | --- | --- | --- | --- |
| syntax-ppss fileio.c c-mode | GNU | 0.34x | 0.60x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'syntax-ppss fileio.c c-mode'` |
| syntax-ppss xdisp.c c-mode | GNU | 0.22x | 0.34x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'syntax-ppss xdisp.c c-mode'` |
| syntax-ppss fileio.c c-ts-mode | GNU | 0.08x | 0.13x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'syntax-ppss fileio.c c-ts-mode'` |
| key in the middle fileio.c c-ts-mode | GNU | 0.36x | 0.40x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'key in the middle fileio.c c-ts-mode'` |
| font-lock-ensure fileio.c c-ts-mode | GNU | 0.69x | 0.71x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'font-lock-ensure fileio.c c-ts-mode'` |
| syntax-ppss xdisp.c c-ts-mode | GNU | 0.02x | 0.03x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'syntax-ppss xdisp.c c-ts-mode'` |
| key in the middle xdisp.c c-ts-mode | GNU | 0.38x | 0.41x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'key in the middle xdisp.c c-ts-mode'` |
| font-lock-ensure xdisp.c c-ts-mode | GNU | 0.73x | 0.75x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'font-lock-ensure xdisp.c c-ts-mode'` |
| micro named call | GNU | 0.53x | 0.53x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'micro named call'` |
| uncompiled arithmetic | GNU | 0.07x | 0.11x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'uncompiled arithmetic'` |
| uncompiled named call | GNU | 0.06x | 0.06x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'uncompiled named call'` |
| uncompiled dolist 10 | GNU | 0.05x | 0.06x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'uncompiled dolist 10'` |
| key self-insert xdisp.c c-ts-mode | GNU | 0.57x | 0.08x | - | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'key self-insert xdisp.c c-ts-mode'` |
| open xdisp.c c-ts-mode | GNU | 0.57x | 0.50x | 0.11x | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'open xdisp.c c-ts-mode'` |
| open 15 MB header c-ts-mode | GNU | 0.50x | 0.55x | 0.07x | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'open 15 MB header c-ts-mode'` |
| theme switch first visit | GNU | 6.56x | 0.62x | 0.64x | `c4f4cebe` | `perfbench.py --romacs BINARY --only 'theme switch first visit'` |
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

## What one frame costs the render thread (`render_stages_live.py`)

The render thread's side of the same scenarios, held to bars. Under
`ROMACS_DIAGNOSTICS=1` the render thread writes a `frame-cost` line for
every frame it draws (the display runtime's `frame_cost`): what it added,
since the frame before, to the renderer's counts (text rows met,
tessellated and reused, glyph records handed to glyph passes, font
identities hashed, bytes uploaded) and the instructions it retired in the
scene's ingestion and the frame's draw, with the part inside the surface's
acquire, the queue's submit and the present apart. The harness runs each
painting scenario of `display_stages.el` and of `render_stages.el` (three
windows with one typed in, a scroll by a line) as `display_stages_live.py`
does, and takes medians over the frames that carried a new scene: its own
instructions are a frame's less the driver's part. Frames drawn with no
new scene (an animation's) are counted apart, per ten steps.
`render_stages_bars.json` holds each scenario's figures, measured on a
software adapter: the rows exactly, every cost under a ceiling. The
compositor, and so Romacs, runs on the host's GPU when
`ROMACS_HARNESS_WLR_ENV` gives it one; a GPU draws the animations a
software adapter is spared, so a GPU run takes its own `--bars`.

```sh
python3 -B crates/romacs-vm-gui/harness/render_stages_live.py --binary BIN --output DIR \
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
