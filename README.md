<!-- perfbench-lead-begin -->
# Romacs

<!-- hero-clip -->

**Emacs, rebuilt for how you work now.**

Romacs is a new machine for Emacs Lisp: a Rust runtime, a GPU-rendered
display, and worker VMs that keep you typing while real work runs beside it.
Your packages, your init and your muscle memory come with you.

### What you're seeing

1. **Two startup screens.** Romacs's logo is drawn live on the GPU, beside
   GNU Emacs as it ships.
2. **A race.** Both editors run the same command, `vc-print-root-log`, on
   Emacs's own repository: every commit since 1985.  Both press `M->` once a
   second to chase the end of the log.  The big number is how far back each
   one has reached.  Romacs gets to the first commit in 2.7 s; GNU Emacs
   needs 18.7 s.
3. **The theme picker.** Each theme applies live as you move through the
   list, before you choose.
4. **Agents.** Claude works in a Romacs chat, hands a review to Codex as a
   subagent, and the header shows it running.  `C-c C-o` opens the chat's
   menu, with every running agent listed.
5. **The install line.**

Every frame is the real program.  The race runs at real speed.  Captions are
added, and scenes are cut and briefly held so you can read them.

## Why you'll switch

### It's fast where you feel it

| What you do | Romacs | GNU Emacs |
| --- | ---: | ---: |
| Log the entire Emacs history, 183,254 commits | **2.8 s** | 17.3 s |
| Open a 15 MB generated C header, faces and all | **1.03 s** | 5.69 s |
| Switch themes | **3.92 ms** | 19.9 ms |
| A typed key reaches the screen, in `xdisp.c` | **11 ms** | 15 ms |
| Open `xdisp.c`, faces and all | 222 ms | **143 ms** |

Keys stay at a median of 4.78 ms while that whole history streams
in.  Romacs parses `xdisp.c` with tree-sitter on open, where GNU Emacs's C mode
uses regexps, and that's the row we're coming for next.  Org and dired are
slower too, and every number, wins and losses, is in the tables below.  These
are the first headlines, and we keep pushing them.  `perfbench` regenerates
them from each run.

### It stays in your hands

Worker VMs run searches, syncs and background loads off the main VM, so you
keep typing.  `C-g` reaches long motion and regexp work, and
even a stuck session can show you its stack, live, over MCP.

### Agents live inside it

Claude and Codex sessions run in the editor: real chats with images,
subagents as cards, a session switcher, and full access to the machine
through Lisp and MCP.  Every buffer, window, process and option is
something an agent can query and change.  No terminal wrapper.  No plugin.
Inspired by T3 Code's orchestration, and T3 Code's providers work here too.

### It looks like this decade

A GPU renderer with animation, a clean status bar, floating completion and
dozens of colour schemes built in: Tokyo Night, Catppuccin, Gruvbox, Nord,
Rosé Pine and more, all readable by design.  On Omarchy, Romacs follows your
desktop theme by itself.

### It works out of the box

Tree-sitter highlighting by default, fzf-style completion, native TLS,
Org and version control.  Open it and start working.

### It's still Emacs

GNU's Lisp runs on it, and GNU's own test suite keeps it honest.
Everything is a Lisp program you can read, change and rebind while it runs.
<!-- perfbench-lead-end -->

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

## Direction

Keep Emacs's malleability without preserving every historical constraint.
Rust handles the machinery; Elisp keeps commands, keybindings and policy
editable without a rebuild. The goal is broad GNU Elisp compatibility,
not a replica of GNU Emacs's internals.

Rust emulates the machine: object memory and collection, the evaluator,
loading, processes, and the display and input host.  Elisp is the runtime:
GNU's Lisp corpus (`gnu-lisp/`) and Romacs's own Lisp (`rlisp/`) run on it
as programs, and compatibility is judged by what those programs do.

A user should be limited only by imagination in what they can do
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
| micro named call | iteration | 10.9 ns | 4.88 ns | 0.45x | 0.59x | (m-id i); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro closure call | iteration | 10.2 ns | 18.7 ns | 1.83x | 1.85x | (funcall f0 i); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro mapcar 1 | iteration | 93.4 ns | 128 ns | 1.37x | 1.45x | (mapcar f0 (cdr (cdr (cdr (cdr (cdr (cdr (cdr (cdr (cdr l)))))))))); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro mapcar 10 | iteration | 673 ns | 643 ns | 0.96x | 0.88x | (mapcar f0 l); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| micro dolist + funcall 10 | iteration | 165 ns | 263 ns | 1.60x | 1.61x | (dolist (x l) (funcall f0 x)); GNU compiled; Romacs: compiled store, 1 of 1 loads compiled |
| ERT 25 files | total | 15.3 s | 19.3 s | 1.26x | 1.36x | GNU 927/929 expected, Romacs 894/929 expected; wall summed per file; Romacs: compiled store, 39 of 42 loads compiled, read as source: cus-load.el, uni-name.el, uni-old-name.el, loads not checked in 9 of 75 runs; test files read as source in both |
| first-start world build | run | - | 2.63 s | - | - | Romacs only: --loadup --world-build in a fresh store; a world build reads source, never the compiled store |
| warm launch to first paint | run | - | 260 ms | - | - | Romacs only (its --trace-latency first-paint); GNU has no paint mark; Romacs: compiled store, 0 of 0 loads compiled |
| warm launch to idle | run | 395 ms | 281 ms | 0.71x | 0.84x | spawn to the first idle after emacs-startup-hook; instructions to exit there; Romacs: compiled store, 0 of 0 loads compiled |
| key self-insert | key | 1.1 ms | 1.36 ms | 1.24x | 0.91x | 40 keys x 3; wall pre-command to idle; samples GNU 120, Romacs 120; Romacs: compiled store, 1 of 1 loads compiled |
| key C-f | key | 123 µs | 1.11 ms | 9.00x | 1.26x | 40 keys x 3; wall pre-command to idle; samples GNU 120, Romacs 120; Romacs: compiled store, 1 of 1 loads compiled |
| key C-n | key | 2.43 ms | 1.45 ms | 0.59x | 0.77x | 40 keys x 3; wall pre-command to idle; samples GNU 120, Romacs 120; Romacs: compiled store, 1 of 1 loads compiled |
| C-x C-f 5,000 files, per key | key | 69.5 ms | 51.8 ms | 0.75x | 0.67x | fido-vertical, keys of file-1; candidates GNU 1000, Romacs 1000; Romacs: compiled store, 1 of 1 loads compiled |
| C-x C-f 5,000 files, per key, romacs-complete | key | - | 2.83 ms | - | - | Romacs only: its default completion (the layer, fzf), keys of file-1; candidates 2084, minibuffer 'file-1'; Romacs: compiled store, 1 of 1 loads compiled |
<!-- perfbench-table-end -->

### Interactive sessions

<!-- perfbench-session-begin -->
Real commands and keys in a GUI session.  Wall runs from the key to the result.  Block is the longest gap between ticks of a 5 ms timer, which cannot run while the main VM is busy: how long the session stopped answering.  Read these rows by wall and block: work moved to a worker VM adds instructions and removes block.

Measured 2026-11-07 on build `19f44056` against the installed GNU Emacs 32.0.50, commit `b28750b8`, package emacs-wayland-git 32.0.50.r183254.gb28750b82.local-1, natively compiled; the inputs are pinned at `45b0f769`, on AMD Ryzen 9 5950X 16-Core Processor, 32 threads, 47 GB.  Each row ran 3 times per engine in private home directories; wall time is the median, and wall times measured above load 8 are left out.  Romacs ran as an installation does: 1931 of the 1981 files of its Lisp compiled ahead of use into its store by `--compile-lisp-tree`, before anything was timed; batch rows booted from a batch world prepared first, which a user's first batch run builds, and a workload's own file loaded compiled.  Each row's note counts the loads the store confirms and names any it does not.  A ratio is Romacs over the other engine: below 1.00x, Romacs is faster.

| Row | Per | GNU | Romacs | GNU block | Romacs block | Romacs/GNU (time) | Romacs/GNU (block) | Romacs/GNU (instructions) | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| dired 5,000 files | run | 129 ms | 281 ms | 126 ms | 281 ms | 2.18x | 2.23x | 3.05x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| occur xdisp.c | run | 1.35 s | 1.87 s | 1.35 s | 1.87 s | 1.39x | 1.39x | 1.64x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| project-find-regexp | run | 412 ms | 571 ms | 411 ms | 59.2 ms | 1.38x | 0.14x | 0.31x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| project-find-file | run | 32.4 ms | 67.7 ms | 32 ms | 13.8 ms | 2.09x | 0.43x | 1.00x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| vc-log 2,000 commits | run | 45.5 ms | 38.6 ms | 18.8 ms | 11.2 ms | 0.85x | 0.60x | 1.20x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| vc-diff xdisp.c | run | 281 ms | 279 ms | 279 ms | 278 ms | 0.99x | 1.00x | 1.55x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| vc-dir | run | 59.9 ms | 48.9 ms | 25.6 ms | 17.7 ms | 0.82x | 0.69x | 1.85x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| M-x shell, one command | run | 6.64 ms | 28.1 ms | 5.82 ms | 28 ms | 4.23x | 4.81x | 1.33x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| org-agenda, a month | run | 2.94 s | 11.4 s | 2.94 s | 11.4 s | 3.86x | 3.86x | 4.09x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| kill-line, per key | key | 1.64 ms | 1.46 ms | 30.9 ms | 6.82 ms | 0.89x | 0.22x | 7.43x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| yank, per key | key | 1.62 ms | 1.64 ms | 30.9 ms | 6.98 ms | 1.01x | 0.23x | 3.32x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| undo, per key | key | 1.11 ms | 1.29 ms | 30 ms | 6.86 ms | 1.16x | 0.23x | 59.96x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| isearch xdisp.c, per key | key | 2.89 ms | 3.47 ms | 26.7 ms | 15.9 ms | 1.20x | 0.60x | 1.61x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| Org self-insert, per key | key | 1.21 ms | 3.05 ms | 31 ms | 81.7 ms | 2.52x | 2.64x | 18.88x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| Org TAB, per key | key | 2.48 ms | 44.2 ms | 33.9 ms | 334 ms | 17.84x | 9.85x | 49.69x | Romacs: compiled store, 202 of 204 loads compiled, read as source: cus-load.el, org-version.el |
| find-file xdisp.c | run | 143 ms | 222 ms | 138 ms | 221 ms | 1.55x | 1.60x | 1.89x | first shown GNU 138 ms, Romacs 221 ms; mode GNU c-mode, Romacs c-ts-mode; Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| find-file generated header | run | 5.69 s | 1.03 s | 4.43 s | 1.03 s | 0.18x | 0.23x | 0.26x | first shown GNU 4.43 s, Romacs 1.03 s; mode GNU c-mode, Romacs c-ts-mode; Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| vc-print-root-log whole history | run | 17.3 s | 2.8 s | 55.6 ms | 37.8 ms | 0.16x | 0.68x | 0.25x | first shown GNU 9.08 ms, Romacs 10.6 ms; median key GNU 4.59 ms, Romacs 4.78 ms; Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more |
| theme switch, first visit | run | 27.7 ms | 17 ms | 26.7 ms | 16.9 ms | 0.61x | 0.63x | 6.26x | Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more; each engine on its own shipped themes |
| theme switch, cycled | run | 19.9 ms | 3.92 ms | 19.3 ms | 3.9 ms | 0.20x | 0.20x | 0.68x | Romacs: compiled store, 36 of 43 loads compiled, read as source: cus-load.el, dracula-theme.el, gruvbox-dark-theme.el, nord-theme.el, solarized-dark-theme.el, tokyonight-night-theme.el and 1 more; each engine on its own shipped themes |
<!-- perfbench-session-end -->

### Headlines

<!-- perfbench-headlines-begin -->
Every row of this run where Romacs beats a rival by at least 20%: 0.80x or better on every metric the run measured.  Build `19f44056`.  The tables keep every row, losses included.

| Row | Against | Instructions | Time | Block | Build | Rerun |
| --- | --- | --- | --- | --- | --- | --- |
| how-many xdisp.c | GNU | 0.10x | 0.16x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'how-many xdisp.c'` |
| micro named call | GNU | 0.59x | 0.45x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'micro'` |
| elb bubble-no-cons | GNU | 0.66x | 0.68x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'elb'` |
| elb fibn | GNU | 0.47x | 0.57x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'elb'` |
| session find-file generated header | GNU | 0.26x | 0.18x | 0.23x | `19f44056` | `perfbench.py --romacs BINARY --only 'session'` |
| session vc-print-root-log whole history | GNU | 0.25x | 0.16x | 0.68x | `19f44056` | `perfbench.py --romacs BINARY --only 'session'` |
| session theme switch, first visit | GNU | 6.26x | 0.61x | 0.63x | `19f44056` | `perfbench.py --romacs BINARY --only 'session'` |
| session theme switch, cycled | GNU | 0.68x | 0.20x | 0.20x | `19f44056` | `perfbench.py --romacs BINARY --only 'session'` |
| key C-n | GNU | 0.77x | 0.59x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'keys'` |
| C-x C-f 5,000 files, per key | GNU | 0.67x | 0.75x | - | `19f44056` | `perfbench.py --romacs BINARY --only 'keys'` |
<!-- perfbench-headlines-end -->

### Lisp compute: GNU's elisp-benchmarks

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

## Source layout

`rlisp/` holds Romacs Lisp, `gnu-lisp/` the pinned GNU sources, and
`rlisp/deps/` temporary third-party dependencies and their replacement plan.

## Build and run

On Linux with a Wayland session, install the Rust toolchain specified in
`rust-toolchain.toml` and the native dependencies in `nix/dependencies.nix`.

From the repository root:

```sh
cargo build --release --locked -p romacs-vm-gui
scripts/romacs
```
