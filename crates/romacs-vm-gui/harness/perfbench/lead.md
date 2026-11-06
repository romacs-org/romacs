# Romacs

<!-- hero-clip -->

**Emacs, rebuilt for how you work now.**

Romacs is a new machine for Emacs Lisp: a Rust runtime, a GPU-rendered
display, and worker VMs that keep you typing while real work runs beside it.
Your packages, your init and your muscle memory come with you.

## Why you'll switch

### It's fast where you feel it

- **The entire Emacs git history in a log buffer:** {{vc-history-commits}}
  commits in {{vc-history-romacs}}, against {{vc-history-gnu}} in GNU Emacs,
  and the median key takes {{vc-history-key}} while it streams in.
- **Big files open fast:** `xdisp.c` with all its faces in
  {{open-xdisp-romacs}}, against {{open-xdisp-gnu}}.  A {{open-big-size}}
  header in {{open-big-romacs}}, against {{open-big-gnu}}.
- **Themes switch in a blink:** {{theme-romacs}} against {{theme-gnu}}, with a
  live preview of every theme before you choose.

These are the first headlines, and we keep pushing them.  `perfbench`
regenerates the numbers from each run, and anything that gets worse stays in
the table below.

### It stays in your hands

Worker VMs run searches, syncs and background loads off the main VM, so you
keep typing.  `C-g` reaches long motion and regexp work, and
even a stuck session can show you its stack, live, over MCP.

### Agents live inside it

Claude and Codex sessions run in the editor: real chats with images,
subagents as cards, a session switcher, and full access to the machine
through Lisp and MCP.  Every buffer, window, process and option is
something an agent can query and change.  No terminal wrapper.  No plugin.

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
