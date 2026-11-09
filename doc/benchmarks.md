# Romacs and GNU Emacs compared

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

## Interactive sessions

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

## Headlines

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

## Lisp compute: GNU's elisp-benchmarks

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
