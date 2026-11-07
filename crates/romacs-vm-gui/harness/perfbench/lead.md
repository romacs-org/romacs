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

| What you do | Romacs | GNU Emacs | GNU with tree-sitter |
| --- | ---: | ---: | ---: |
| Log the entire Emacs history, {{vc-history-commits}} commits | **{{vc-history-romacs}}** | {{vc-history-gnu}} | |
| Open a {{open-big-size}} generated C header, faces and all | **{{open-big-romacs}}** | {{open-big-gnu}} | {{open-big-ts-gnu}} |
| Open `xdisp.c`, faces and all | {{open-xdisp-romacs}} | **{{open-xdisp-gnu}}** | {{open-xdisp-ts-gnu}} |
| Switch themes | **{{theme-romacs}}** | {{theme-gnu}} | |
| A typed key reaches the screen, in `xdisp.c` | **11 ms** | 15 ms | |
| Uncompiled Lisp, one arithmetic step | **{{uncompiled-arith-romacs}}** | {{uncompiled-arith-gnu}} | |
| Uncompiled Lisp, one function call | **{{uncompiled-call-romacs}}** | {{uncompiled-call-gnu}} | |
| Uncompiled Lisp, a 10-item list loop | **{{uncompiled-list-romacs}}** | {{uncompiled-list-gnu}} | |

Tree-sitter is the default, so C files use it out of the box: Romacs is about
twice as fast as GNU Emacs's tree-sitter mode, and GNU's regexp C mode still
opens `xdisp.c` quicker.  GNU shows its first text after about 25 ms there,
where Romacs shows the file once it's parsed.

**No compile step needed.**  Your init, `M-:` and anything
uncompiled run 8 to 17 times faster than in GNU's interpreter, with no compile
step.  Our target is native-compilation speed for all of it.  We're not there
yet: on pure compute, compiled GNU Lisp is still ahead, and the benchmarks
below show by how much.

**More than one core.**  GNU Emacs does its work on one.  Romacs's whole-history
log uses 2.4 cores, and worker VMs take searches off the main VM: during
`project-find-regexp` it's blocked for 59 ms, against 432 ms in GNU Emacs,
though the search itself takes a little longer today.

**Smooth is the goal, and this is early development.**  Keys stay at a median
of {{vc-history-key}} while the whole history streams in.  Garbage collection
is the honest gap: under heavy allocation Romacs spends a third of GNU's time
collecting, but in longer pauses, 65 ms against 5 ms.  Org and dired are slower
too.  Every number, wins and losses, is in the tables below, and `perfbench`
regenerates them from each run.

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
