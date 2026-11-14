# Temporary Lisp dependencies

These packages stay here while Romacs replaces them with its own implementations.
They are dependencies, not GNU sources or Romacs-authored code. Their original
license and attribution files remain with them.

| Dependency | Used for | Replacement plan |
| --- | --- | --- |
| websocket | WebSocket connections, including emacs-hermes | A Rust transport module with a small Elisp API for connection events and messages. |
| fsm | Jabber's state-machine support | Small, explicit state transitions in Romacs-owned code. |
| markdown-mode | Markdown editing and rendering helpers | Romacs-owned editing commands, using a Rust parser where useful. |

Replace one dependency at a time. Keep its callers working, test connection and
cleanup behavior where applicable, and remove the bundled package only when the
replacement covers its actual uses. Replacements should be independent
implementations, not copies with the original notices removed.

This is a development plan, not a claim that these replacements already exist.
