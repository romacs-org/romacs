## Input and GC boundary

GNU's circular face-ref lists and self-containing `:inherit` plists recurse
in C until SIGSEGV; a circular plist tail hangs. We reject them during the
iterative graph conversion. Every public face-data input requires a
`BoundedDatum`: depth at most 32, at most 2^20 cons cells, and at most 16 MiB
of **expanded** UTF-8 string/symbol payload. The fixture reader also limits
printed source bytes and quote frames. These limits deliberately reject
otherwise valid but unreasonable inputs. Borrowed graph atoms are budgeted
before copying, so sharing one large string cannot amplify without bound.

Raw `Datum` is an untrusted construction language, not an accepted face
input. Its owner remains responsible for disposing of an arbitrarily deep
raw tree iteratively. `BoundedDatum::new` checks by reference before cloning.
`FaceRef` is opaque, and structured attribute values also require the checked
type; neither offers an unchecked route back into recursive face processing.

The core adapter must hold **`&Context` for the whole conversion**. Tagged
`Value` reads do not themselves borrow the heap or inhibit GC. Its `view`
callback must not allocate, intern, execute Lisp or reach a GC safe point.
Only the converter allocates native owned storage. No Lisp handle escapes.
`NodeView::Symbol` means an interned name; the reader must report uninterned
symbols as unsupported, not conflate their identities. There is no core
adapter in this crate, so the actual Context/GC integration still needs its
core-side test when wired; a toy heap test would not prove that contract.

Tests: `cycles.rs`, `readiness.rs` quote/raw-million-node subprocesses,
byte budgets, borrowed-atom budget, and `ui/raw_{defface,remap}.rs`.
