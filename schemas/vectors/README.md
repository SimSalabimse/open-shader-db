# Upload-manifest golden vectors

Shared fixtures for `crates/upload` and `worker/`. Each `*.json` file
is one case.

| Field | Meaning |
| --- | --- |
| `verdict` | `accept` or `reject` |
| `error_code` | Worker / `Error::code()` string, or `null` on accept |
| `body` | JSON value; encoded compact with sorted keys for the parser |
| `body_text` | Exact UTF-8 payload (duplicate keys, trailing tokens) |
| `body_hex` | Exact bytes, lowercase hex (BOM, non-UTF-8) |

A file uses exactly one of `body`, `body_text`, or `body_hex`.

Valid `body` values are produced by `osdb-upload` (canonical
`UploadManifest`). Invalid cases are hand-written. Both language test
suites walk this directory and run every file. The JSON Schema is
**not** the oracle; several reject cases are schema-valid.
