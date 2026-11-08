# Romacs

<a href="https://thanosapollo.org/romacs/romacs.mp4"><img src="https://thanosapollo.org/romacs/romacs.webp" alt="Romacs and GNU Emacs logging Emacs's whole history side by side"></a>

Emacs Lisp on a new machine: a Rust runtime, a GPU-rendered display, and worker
VMs that keep you typing while real work runs beside it.  Your packages, your
init and your muscle memory come with you.

| | Romacs | GNU Emacs |
| --- | ---: | ---: |
| Log the entire Emacs history, {{vc-history-commits}} commits | **{{vc-history-romacs}}** | {{vc-history-gnu}} |
| Open a {{open-big-size}} C header, tree-sitter in both | **{{open-big-romacs}}** | {{open-big-ts-gnu}} |
| Open `xdisp.c`, tree-sitter in both | **{{open-xdisp-romacs}}** | {{open-xdisp-ts-gnu}} |
| Switch themes | **{{theme-romacs}}** | {{theme-gnu}} |
| Uncompiled Lisp, one function call | **{{uncompiled-call-romacs}}** | {{uncompiled-call-gnu}} |

The clip is the real program; the race runs at real speed.  Every number, losses
included, is in [doc/benchmarks.md](doc/benchmarks.md).

- **Worker VMs.**  Searches, syncs and background loads run off the main VM, and
  `C-g` reaches long work.
- **Agents.**  Claude and Codex in real chats, with every buffer, window and
  process open to them through Lisp and MCP.  Inspired by T3 Code's orchestration,
  and T3 Code's providers work here too.
- **GPU rendering.**  Animation, floating completion and dozens of colour schemes.
  On Omarchy it follows your desktop theme.
- **Still Emacs.**  GNU's Lisp runs on it, and GNU's own test suite keeps it honest.
