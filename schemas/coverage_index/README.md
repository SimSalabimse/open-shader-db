# Coverage index golden files

Fixtures for `osdb-coverage`. These files are not under `schemas/vectors/`
because that directory is the upload-manifest set walked by the Worker.

`valid/` files are canonical bytes (sorted keys, no trailing newline).
`loadable/v1-unsorted.json` is format version 1 with hashes out of order.
The loader sorts version 1 hashes. Version 2 does not.
`invalid/` files must be rejected. The 60 MiB unknown-key case stays a
library test so this tree does not carry a huge file.
`invalid/v2-overlong-string.json` is a shorter hostile string (800 bytes)
that must fail before the string is copied.

The JSON Schema files `schemas/coverage_index.v1.json` and
`schemas/coverage_index.v2.json` describe the two on-disk shapes. The
Rust loader is the oracle. Format version 1 hashes are SODB description
hashes. Format version 2 hashes are pipeline content hashes.
