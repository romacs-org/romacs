# crates/romacs-vm/src/markdown
Romacs's one Markdown mechanism: pulldown-cmark parses CommonMark with GFM
tables, strikethrough and task lists, and this module turns its events into
face runs, block structure and table grids.

## Boundary
- Functions here are pure over a `&str`, and positions are byte offsets into
  it. Buffers, character positions and property writes belong to
  `crates/romacs-vm/src/prim/leaf/markdown.rs`, the primitives to
  `crates/romacs-vm/src/prims/markdown.rs`.
- Faces, and which kind of element gets which (`romacs-markdown-faces`),
  belong to `lisp/romacs/romacs-markdown.el`.
- The dialect is `spans::options()` alone.
- Markdown anywhere in Romacs goes through here: no second parser or regexp
  fontifier for it.

## Contracts
- `restart`: text appended to a document never changes its runs before that
  point, so a streamed reply is refaced from there after each chunk. A link
  reference definition that arrives late is the one exception; a stream's
  last call refaces the document whole.
- Runs are ascending and disjoint, and each run's kinds come innermost first,
  as a `face` list orders them; markup is innermost.
- The face writes are silent: `romacs-markdown--fontify` refuses to run
  unless modification hooks are inhibited and undo is off.

## Tests
- `python3 tools/dev-loop romacs-vm --profile gate -E 'test(markdown)'`.
- On a built host, `python3 tools/test-ert --file
  test/romacs/lisp/romacs-markdown-tests.el --file
  test/romacs/lisp/romacs-agent-markdown-tests.el`.
- A new kind of element: a `Kind` variant in `Kind::ALL`, its default face
  in `lisp/romacs/romacs-markdown.el`, and a test by kind.

## Traps
- A partial line can turn the block it starts into the previous one's (a
  `1` that becomes `1. item` continuing a list, a setext underline), so
  `restart` steps back one block past the one holding the change.
- GFM splits table cells at every unescaped pipe, inside code spans too.

## Decisions
- pulldown-cmark over tree-sitter-md: tree-sitter-md agreed with the spec
  examples on 89.6% and cost about 1G instructions a key on a 1 MB file.
  pulldown-cmark passes the examples and reparses only the changed blocks.
