## Structured marker and overlay results are opaque type tags

The agent `eval` entry point serializes markers as `{"type":"marker"}` and
overlays as `{"type":"overlay"}`, without signaling. This follows the existing
bare `buffer`, `byte-buffer`, and `function` convention for known, unreadable,
identity-bearing values. Unknown tag classes still signal an unsupported result.

GNU prints these live objects as `#<marker at 1 in foo>`,
`#<overlay from 1 to 5 in foo>`, and `#<buffer foo>` respectively; it returns
values, not errors. The structured protocol deliberately omits position, buffer,
and identity payloads. Moving mark/compact GC relocates Value indices, so no
index is exposed as a reusable reference. Reading positions here would also
couple serialization to anchor state. This is an extensible protocol choice:
additional object fields can be specified later without removing existing data.
