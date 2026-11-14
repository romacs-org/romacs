# crates/romacs-vm/src/markdown
Romacs's one Markdown mechanism: pulldown-cmark parses CommonMark with GFM
tables, strikethrough and task lists, and this module turns its events into
face runs, block structure and table grids, and keeps a buffer's block index.

## Boundary
- Functions here are pure over a `&str`, and positions are byte offsets into
  it. Buffers, character positions and property writes belong to
  `crates/romacs-vm/src/prim/leaf/markdown.rs`, the primitives to
  `crates/romacs-vm/src/prims/markdown.rs`.
- Faces, and which kind of element gets which (`romacs-markdown-faces`),
  belong to `lisp/romacs/romacs-markdown.el`; the editing mode is
  `lisp/romacs/romacs-markdown-mode.el`.
- A buffer's `Index` lives on its text (`crates/romacs-vm/src/text.rs`,
  `edit_derived`), empty until Markdown is asked of it; it holds no text.
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
- The face writes are silent: the fontify primitives refuse to run unless
  modification hooks are inhibited and undo is off.
- `Index::sync` leaves the block starts a fresh read of the text finds
  (`index_follows_edits`); what it read again outside the asked blocks
  loses `fontified`, so redisplay asks for it.

## Tests
- `python3 tools/dev-loop romacs-vm --profile gate -E 'test(markdown)'`.
- On a built host, `python3 tools/test-ert --file
  test/romacs/lisp/romacs-markdown-tests.el --file
  test/romacs/lisp/romacs-markdown-mode-tests.el --file
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
  pulldown-cmark passes the examples. A region is parsed whole and `restart`
  limits which faces are written again; a buffer is parsed by the blocks its
  index says hold the asked region.
