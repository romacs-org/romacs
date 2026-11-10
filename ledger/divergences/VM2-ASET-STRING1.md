## Observable by a package

| ID | Area | GNU behavior | Romacs behavior |
| --- | --- | --- | --- |
| VM2-ASET-STRING1 | `aset` of a non-ASCII character into a multibyte string | Pinned `Faset` (`src/data.c`) refuses a non-ASCII character in a multibyte string. Romacs stores it. | Romacs's ASCII strings are multibyte where GNU's are unibyte, and GNU stores a byte in those, so refusing here would refuse stores GNU allows. | `tests/byte_op_kernels.rs` keeps both cases listed. |
