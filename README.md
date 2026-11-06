<!-- perfbench-lead-begin -->
# Romacs

<!-- hero-clip -->

Romacs is a Lisp machine emulator written in Rust, with Emacs Lisp as its
runtime.  People and agents program the whole machine in Lisp; editing is one
application running on it.
<!-- perfbench-lead-end -->

## Direction

Keep Emacs's malleability without preserving every historical constraint.
Rust handles the machinery; Elisp keeps commands, keybindings and policy
editable without a rebuild. The goal is broad GNU Elisp compatibility,
not a replica of GNU Emacs's internals.

Rust emulates the machine: object memory and collection, the evaluator,
loading, processes, and the display and input host.  Elisp is the runtime:
GNU's Lisp corpus (`gnu-lisp/`) and Romacs's own Lisp (`rlisp/`) run on it
as programs, and compatibility is judged by what those programs do.

A user should be limited only by his imagination by what he can do
with romacs.

Minibuffer completion matches the way fzf does by default: fuzzy terms in
any order, `^prefix`, `suffix$`, `'exact`, `!not` and `a | b`, best match
first.  `(setopt completion-styles '(basic partial-completion emacs22) completion-category-overrides nil)` restores GNU's (DIVERGENCES.md VM2-FZF-DEFAULT).

Mail is the real notmuch package, loaded from the user's init, on GNU's
message.el and smtpmail over Romacs's native TLS; Romacs ships no mail
interface of its own (DIVERGENCES.md VM2-MAIL1).  A distribution's notmuch
installs its Lisp in `/usr/share/emacs/site-lisp`, which the init adds to
`load-path` (VM2-SITELISP1).

## Romacs and GNU Emacs compared

Performance against GNU Emacs as a user installs it, measured with the standing benchmark
suite (`crates/romacs-vm-gui/harness/perfbench/`).  Romacs runs as an
installation runs it, its Lisp compiled ahead of use.  The tables are regenerated
from each new run, losses included.  To reproduce them:

```sh
python3 -B crates/romacs-vm-gui/harness/perfbench/perfbench.py --romacs BINARY
python3 -B crates/romacs-vm-gui/harness/perfbench/readme.py OUT/COMMIT/TIME/results.json
```

<!-- perfbench-table-begin -->
Measured 2026-11-06 on build `0cfcb823` against pinned GNU Emacs `45b0f769`, on AMD Ryzen 9 5950X 16-Core Processor, 32 threads, 47 GB.  Each workload ran 3 times per engine in private home directories; wall time is the median, and wall times measured above load 8 are left out.  Romacs ran as an installation does: 1920 of the 1970 files of its Lisp compiled ahead of use into its store by `--compile-lisp-tree`, before anything was timed; batch rows booted from a batch world prepared first, which a user's first batch run builds, and a workload's own file loaded compiled.  Each row's note counts the loads the store confirms and names any it does not.  The ratio is Romacs over GNU: below 1.00x, Romacs is faster.

| Workload | Per | GNU | Romacs | Romacs/GNU (time) | Romacs/GNU (instructions) | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| startup -Q --batch | run | - | - | - | 1.57x | wall refused: load 10.2 > 8.0; process to exit; Romacs: compiled store, 0 of 0 loads compiled |
| font-lock-ensure fileio.c | run | - | - | - | 3.31x | wall refused: load 10.2 > 8.0; less its setup; Romacs: compiled store, 10 of 10 loads compiled |
| font-lock-ensure subr.el | run | - | - | - | 3.38x | wall refused: load 10.3 > 8.0; less its setup; Romacs: compiled store, 0 of 0 loads compiled |
| font-lock-ensure ORG-NEWS | run | - | - | - | 4.49x | wall refused: load 14.5 > 8.0; less its setup; Romacs: compiled store, 169 of 170 loads compiled, read as source: org-version.el |
| dired 5,000 files | run | - | - | - | 4.54x | wall refused: load 18.3 > 8.0; less its setup; Romacs: compiled store, 3 of 3 loads compiled |
| project-find-regexp repo | run | - | - | - | 2.02x | wall refused: load 22.8 > 8.0; less its setup; Romacs: compiled store, 25 of 26 loads compiled, read as source: cus-load.el |
| vc-log 2,000 commits | run | - | - | - | 3.06x | wall refused: load 25.1 > 8.0; less its setup; Romacs: compiled store, 14 of 15 loads compiled, read as source: cus-load.el |
| occur xdisp.c | run | - | - | - | 2.02x | wall refused: load 26.4 > 8.0; less its setup; Romacs: compiled store, 0 of 0 loads compiled |
| how-many xdisp.c | run | - | - | - | 0.14x | wall refused: load 26.8 > 8.0; less its setup; Romacs: compiled store, 0 of 0 loads compiled |
| micro named call | iteration | - | - | - | 0.53x | wall refused: load 28.2 > 8.0; (m-id i); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro closure call | iteration | - | - | - | 1.95x | wall refused: load 28.2 > 8.0; (funcall f0 i); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro mapcar 1 | iteration | - | - | - | 2.28x | wall refused: load 28.2 > 8.0; (mapcar f0 (cdr (cdr (cdr (cdr (cdr (cdr (cdr (cdr (cdr l)))))))))); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro mapcar 10 | iteration | - | - | - | 1.12x | wall refused: load 28.2 > 8.0; (mapcar f0 l); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro dolist + funcall 10 | iteration | - | - | - | 1.73x | wall refused: load 28.2 > 8.0; (dolist (x l) (funcall f0 x)); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| ERT 25 files | total | - | - | - | 2.23x | wall refused: load 31.7 > 8.0; GNU 927/929 expected, Romacs 890/929 expected; wall summed per file; Romacs: compiled store, 39 of 42 loads compiled, read as source: cus-load.el, uni-name.el, uni-old-name.el, loads not checked in 9 of 75 runs; test files read as source in both |
| first-start world build | run | - | - | - | - | wall refused: load 42.6 > 8.0; Romacs only: --loadup --world-build in a fresh store; a world build reads source, never the compiled store |
| warm launch to first paint | run | - | - | - | - | wall refused: load 42.6 > 8.0; Romacs only (its --trace-latency first-paint); GNU has no paint mark; Romacs: compiled store, 0 of 0 loads compiled |
| warm launch to idle | run | - | - | - | 0.96x | wall refused: load 42.6 > 8.0; spawn to the first idle after emacs-startup-hook; instructions to exit there; Romacs: compiled store, 0 of 0 loads compiled |
| key self-insert | key | - | - | - | 1.29x | wall refused: load 36.3 > 8.0; 40 keys x 3; wall pre-command to idle; samples GNU 120, Romacs 120; Romacs: compiled store, 1 of 1 loads compiled |
| key C-f | key | - | - | - | 1.81x | wall refused: load 36.3 > 8.0; 40 keys x 3; wall pre-command to idle; samples GNU 120, Romacs 120; Romacs: compiled store, 1 of 1 loads compiled |
| key C-n | key | - | - | - | 1.53x | wall refused: load 36.3 > 8.0; 40 keys x 3; wall pre-command to idle; samples GNU 120, Romacs 120; Romacs: compiled store, 1 of 1 loads compiled |
| C-x C-f 5,000 files, per key | key | - | - | - | 0.96x | wall refused: load 36.3 > 8.0; fido-vertical, keys of file-1; candidates GNU 1000, Romacs 1000; Romacs: compiled store, 1 of 1 loads compiled |
| C-x C-f 5,000 files, per key, romacs-complete | key | - | - | - | - | wall refused: load 36.3 > 8.0; Romacs only: its default completion (the layer, fzf), keys of file-1; candidates 2084, minibuffer 'file-1'; Romacs: compiled store, 1 of 1 loads compiled |

Not measured in this run: init, gnosis, apps.
<!-- perfbench-table-end -->

### Interactive sessions

<!-- perfbench-session-begin -->
<!-- perfbench-session-end -->

### Headlines

<!-- perfbench-headlines-begin -->
Every row of this run where Romacs beats a rival by at least 20%: 0.80x or better on every metric the run measured.  Build `0cfcb823`.  The tables keep every row, losses included.

| Row | Against | Instructions | Time | Build | Rerun |
| --- | --- | --- | --- | --- | --- |
| how-many xdisp.c | GNU | 0.14x | - | `0cfcb823` | `perfbench.py --romacs BINARY --only 'how-many xdisp.c'` |
| micro named call | GNU | 0.53x | - | `0cfcb823` | `perfbench.py --romacs BINARY --only 'micro'` |
| elb fibn | GNU | 0.49x | - | `0cfcb823` | `perfbench.py --romacs BINARY --only 'elb'` |
| elb fibn-named-let | GNU | 0.55x | - | `0cfcb823` | `perfbench.py --romacs BINARY --only 'elb'` |
<!-- perfbench-headlines-end -->

### Lisp compute: GNU's elisp-benchmarks

<!-- perfbench-elb-begin -->
This is GNU's own benchmark suite, GNU ELPA's elisp-benchmarks (the one GNU uses to evaluate native compilation), not ours, and it measures pure Lisp compute, not editor responsiveness.

Measured 2026-11-06 on build `0cfcb823` against pinned GNU Emacs `45b0f769`, on AMD Ryzen 9 5950X 16-Core Processor, 32 threads, 47 GB.  Each member ran 3 times per engine in private home directories; wall time is the median, and wall times measured above load 8 are left out.  GNU loads the member files byte-compiled, with JIT native compilation off, and natively compiled at the suite's own speed and safety; its own Lisp libraries are natively compiled in both columns, as this GNU is built with native compilation.  Romacs ran as an installation does: 1920 of the 1970 files of its Lisp compiled ahead of use into its store by `--compile-lisp-tree`, before anything was timed; batch rows booted from a batch world prepared first, which a user's first batch run builds, and a workload's own file loaded compiled.  Each row's note counts the loads the store confirms and names any it does not.  A member an engine does not define, or whose run reported an error, has no figure there, and the geometric mean leaves it out.  The ratio is Romacs over GNU: below 1.00x, Romacs is faster.

| Member | GNU .elc | GNU native | Romacs | Romacs/GNU .elc (time) | Romacs/GNU native (time) | Romacs/GNU .elc (instructions) | Romacs/GNU native (instructions) | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| bubble | - | - | - | - | - | 1.32x | 5.75x | wall refused: load 34.6 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| bubble-no-cons | - | - | - | - | - | 4.24x | 28.52x | wall refused: load 35.5 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| bytecomp | - | - | - | - | - | - | - | wall refused: load 24.8 > 8.0; Romacs: reported "Error: Symbol’s function definition is void: read-positioning-symbols"; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| dhrystone | - | - | - | - | - | 3.83x | 5.96x | wall refused: load 29.6 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| eieio | - | - | - | - | - | 4.24x | 4.25x | wall refused: load 27.9 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| fibn | - | - | - | - | - | 0.49x | 243009.17x | wall refused: load 25.3 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| fibn-named-let | - | - | - | - | - | 0.55x | 0.99x | wall refused: load 23.2 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| fibn-rec | - | - | - | - | - | 1.57x | 783762.19x | wall refused: load 21.4 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| fibn-tc | - | - | - | - | - | 1.50x | 1043.70x | wall refused: load 20.5 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| flet | - | - | - | - | - | 3.94x | 21.03x | wall refused: load 25.9 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| font-lock | - | - | - | - | - | 2.74x | 2.75x | wall refused: load 27.4 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| inclist | - | - | - | - | - | 0.94x | 12.19x | wall refused: load 27.6 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| inclist-type-hints | - | - | - | - | - | 0.94x | 24.37x | wall refused: load 24.0 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| listlen-tc | - | - | - | - | - | 1.93x | 41.71x | wall refused: load 27.3 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| map-closure | - | - | - | - | - | 5.07x | 5.07x | wall refused: load 33.8 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| nbody | - | - | - | - | - | 4.36x | 6.63x | wall refused: load 44.1 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| pack-unpack | - | - | - | - | - | 5.67x | 5.89x | wall refused: load 37.9 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| pack-unpack-old | - | - | - | - | - | 6.65x | 6.65x | wall refused: load 38.5 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| pcase | - | - | - | - | - | 5.83x | 12.94x | wall refused: load 34.8 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| pidigits | - | - | - | - | - | - | - | wall refused: load 35.7 > 8.0; Romacs: refused: romacs: no result from elb: 'Wrong type argument: numberp, 56803950677996156250' (see logs) |
| scroll | - | - | - | - | - | - | - | wall refused: load 34.1 > 8.0; Romacs: no elb-scroll-entry after loading the suite |
| smie | - | - | - | - | - | 3.90x | 3.92x | wall refused: load 45.2 > 8.0; Romacs: compiled store, 65 of 66 loads compiled, read as source: org-version.el |
| geometric mean | - | - | - | - | - | 2.42x | 31.98x | wall refused: load 45.2 > 8.0; over the 19 of 22 members every engine ran; left out: bytecomp, pidigits, scroll |
<!-- perfbench-elb-end -->

## Source layout

`rlisp/` holds Romacs Lisp, `gnu-lisp/` the pinned GNU sources, and
`rlisp/deps/` temporary third-party dependencies and their replacement plan.

## Install

```sh
curl -fsSL https://thanosapollo.org/romacs/install.sh | bash
```

Romacs runs on x86_64 Linux with glibc 2.39 or newer, such as Ubuntu 24.04,
Debian 13 and Arch, in a Wayland session with a Vulkan driver.  The installer
checks the download against its `SHA256SUMS`, installs under `~/.local`, and
asks before using `sudo`, only to add missing system libraries.

Options go after `bash -s --`.  `--nightly` installs the rolling build of
`master`, and `--uninstall` removes Romacs and leaves your init and data:

```sh
curl -fsSL https://thanosapollo.org/romacs/install.sh | bash -s -- --nightly
curl -fsSL https://thanosapollo.org/romacs/install.sh | bash -s -- --uninstall
```

## Build and run

On Linux with a Wayland session, install the Rust toolchain specified in
`rust-toolchain.toml` and the native dependencies in `nix/dependencies.nix`.
Alternatively, enter the development environment with `nix develop`.

From the repository root:

```sh
cargo build --release --locked -p romacs-vm-gui
scripts/romacs
```
