# Markdown (`crates/romacs-vm/src/markdown/`)

Romacs's one Markdown mechanism. pulldown-cmark (pinned in
`crates/romacs-vm/Cargo.toml`) parses; this module turns its events into
what fontifying and editing need. Buffer glue is `src/prim/leaf/markdown.rs`,
the primitives `src/prims/markdown.rs`, the faces and their policy
`lisp/romacs/romacs-markdown.el`.

- Functions here are pure over a `&str`, and positions are byte offsets into
  it. Character positions, buffers and property writes belong to the leaf;
  faces and which kind gets which (`romacs-markdown-faces`) belong to Lisp.
- The dialect is `spans::options()` alone: CommonMark with GFM tables,
  strikethrough and task lists.
- A new kind of element: a `Kind` variant (its name and its place in
  `Kind::ALL`), its default face in `romacs-markdown.el`, and a test by kind
  in `markdown_tests.rs`.
- Streaming rests on `restart`: text appended to a document never changes its
  runs before that point (`appended_text_leaves_runs_before_restart`). A link
  reference definition that arrives late is the one exception; a stream's
  last call refaces it whole.
- Markdown elsewhere goes through here; don't add a second parser or regexp
  fontifier for it.
- Tests: `python3 tools/dev-loop romacs-vm --profile gate -E 'test(markdown)'`,
  and `python3 tools/test-ert --file test/romacs/lisp/romacs-markdown-tests.el
  --file test/romacs/lisp/romacs-agent-markdown-tests.el` on a built host.
