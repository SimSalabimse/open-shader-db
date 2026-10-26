# Staging loader and threshold consensus

Library only: `crates/consensus`, package `osdb-consensus`. It loads a
staging tree and tallies hashes. It does not wire `osdb-cli`, write a
coverage index, publish records, or change the Worker. Those are M3-B2
(publish writers, coverage format v2, CLI) and later, and they are not
done. Nothing in this crate is deployed.

The numbers below are hashes and counts. They do not show that uploads
came from different people, machines, GPUs, or drivers. There are no
accounts.

## Staging layout

The staging root is the directory that contains `game_id` folders. For a
reviewed pull request that is `staging/`. For a worker prefix it is the
key root from [docs/WORKER_DESIGN.md](WORKER_DESIGN.md). The loader does
not add a `staging/` component of its own.

```text
<game_id>/<version>--<exe_hash>/<64 lowercase hex>.json
```

`exe_hash` is 64 lowercase hex. The last `--` in the build directory name
is the split, so `version` may itself contain `--`. Directory components
use the same checks as `worker/src/keys.ts`: 1 to 128 bytes, not `.` or
`..`, no `/`, `\`, `:`, NUL, whitespace, or non-ASCII. A hostile name
cannot leave its parent.

The file name, without `.json`, is the SHA-256 of the file bytes. Those
bytes must parse with `osdb-upload`'s `parse_with_limits` and re-encode
to the same canonical bytes. The manifest `build_id` must equal the
directory (`game_id`, `version`, `exe_hash`). Identical canonical bytes
appear once. Loaded objects are sorted by that hash.

`list_builds` returns build entries that pass those rules and are real
directories. It does not open files. A symlink or a plain file with a
build-shaped name is counted toward the entry cap and is not listed.
`load_build` reads one build. A missing build directory is an empty
report. The functions do not create, rename, or delete anything under
the root.

## Walk bounds

Depth is fixed at two levels under the root: `game_id`, then the build
directory. A directory inside a build directory is not another build and
is not entered.

| Limit | Default | What happens at the cap |
| --- | ---: | --- |
| `max_files_per_build` | 64 | Further candidates are counted as `over_file_cap` and are not opened. The 64 are the lowest file names, which are the claimed object hashes. |
| `max_file_bytes` | 64 MiB | The file is counted as `too_large`. The body is not read. 64 MiB matches the local manifest cap so a reviewed pull request can hold a full manifest. |
| `max_entries_scanned` | 8192 | `list_builds` counts game-level and build-level names. `load_build` counts names inside the one build directory. Crossing the cap is an error. The result is not a shorter list. |
| Manifest caps | `osdb-upload` defaults | Forwarded to parse and canonical re-encode. |

A zero file cap counts every candidate as over the cap and opens nothing.
A zero entry cap is an error as soon as a directory has one name. An
empty directory still succeeds. Callers with a larger tree pass a higher
entry cap. The library does not raise it on its own.

Symlinks are not followed. A symlink at the root, the game component, or
the build component is an error for `load_build` (an empty report would
hide it). `list_builds` counts a symlink or a plain file at build level
and does not list it. Unix opens use `O_NOFOLLOW`, `O_NONBLOCK`, and
`O_NOCTTY`, then require a regular file on the handle, same idea as
`osdb-coverage` and `osdb-records`. A FIFO must not block the process.
On targets without `O_NOFOLLOW`, a symlink can replace the path after the
metadata check. The opened handle is still required to be a regular file,
and the file name must equal the SHA-256 of the bytes that were read.

Check order for one build-directory entry: symlink, non-regular file,
bad file name, too large, over the file cap, then open and capped read,
content hash, parse, canonical bytes, build id. The first match is the
only reason counted. A per-file I/O error is `io` and the rest of the
build is still loaded. Directory-level failures stay errors. Reasons are
counts. Paths and file bytes are not returned.

| Reason | When |
| --- | --- |
| `not_regular_file` | Directory, FIFO, or other non-regular file |
| `symlink` | Symlink, including a swap caught by `O_NOFOLLOW` |
| `bad_file_name` | Not `<64 lowercase hex>.json`, or not a single path component |
| `name_hash_mismatch` | The name is 64 lowercase hex but is not the SHA-256 of the bytes |
| `not_canonical` | Parsed, but the bytes are not the canonical encoding |
| `parse:<code>` | `osdb-upload` rejected the bytes. `<code>` is `Error::code()` |
| `build_id_mismatch` | Canonical manifest names a different build than the directory |
| `too_large` | Larger than `max_file_bytes` |
| `over_file_cap` | Past `max_files_per_build` after sorting names |
| `io` | Opening or reading that file failed. Other files in the build are still loaded |

`StagingRoot`'s `Debug` text is `<redacted>`. Error text does not include
a path or a file body.

`IngestPath` is an argument (`WorkerStaging` or `ReviewedPullRequest`).
The loader stamps it on every loaded object. It does not guess from the
directory name.

## Agreement units

Three lists are tallied. The pipeline unit is the one that stands for a
full pipeline-state record. It is not an application key.

| Unit | Manifest field | Who is in the denominator |
| --- | --- | --- |
| `pipeline_hash` | `pipeline_hashes` (schema version 2) | Counted uploads that have the field, including an empty list |
| `bytecode_hash` | `bytecode_hashes` | Every counted upload |
| `descriptor_key` | `description_hashes` | Every counted upload |

`description_hashes` are descriptor content hashes (root signature, input
layout, stream output), not app keys. `pipeline_hashes` entries are
`pipeline_content_hash` values. See [docs/UPLOAD_MANIFEST.md](UPLOAD_MANIFEST.md).

A version 1 manifest has no `pipeline_hashes`. It does not add pipeline
hashes and it is not in the pipeline denominator. It still counts for
bytecode and descriptor tallies. An empty version 2 list is different:
the upload carries the unit and raises the denominator without adding
hashes.

Each counted upload adds a hash at most once, because
`osdb_upload::HashSet` is sorted and unique.

`build_consensus` also collapses the input by object hash before any
tally. The same canonical object passed several times is one upload, so
it cannot meet `min_uploads` on its own. The same object hash on both
`WorkerStaging` and `ReviewedPullRequest` is still one upload. A copy
that is counted is kept; held copies of that hash are not also returned.
`upload_count` is the number of distinct object hashes.

## Threshold

`AgreementThreshold::new` rejects `min_uploads` of 0 and
`min_ratio_per_mille` above 1000. The default is 3 and 500 (50%). Fields
are private so a struct literal cannot skip the check. A ratio of 0 is
valid and means the count alone decides. The supported default is 500.

`build_consensus` takes the build id, the uploads, the threshold, and a
counting policy. The build id is an argument so an empty slice still has
one, and so the result does not depend on slice order. An upload for a
different build is held with `build_id_mismatch` and is not tallied.

The library uses the threshold it is given. It does not lower either
number when few uploads were counted. One upload cannot meet the default
bar of 3, and repeating that upload does not change the count.

A hash is accepted only when both of these hold, in integer arithmetic:

- `uploads >= min_uploads`
- `uploads * 1000 >= min_ratio_per_mille * denominator`

`uploads` and `denominator` are `u32`. The ratio is at most 1000. Both
products fit in `u64`. `checked_mul` still fails closed if a product
cannot be represented: the hash is not accepted. `denominator` is the
counted uploads that carry that unit, not the raw slice length.
`denominator` of 0 accepts nothing.

Accepted hashes are sorted by unit (`bytecode_hash`, `descriptor_key`,
`pipeline_hash`) and then by the hash. Each accepted row includes how
many counted uploads listed it. Rejected hashes are a count per unit.
The rejected hashes are not listed.

Reported upload totals saturate at `u32::MAX`. Per-hash counts saturate
the same way. That cap is not a reason to lower the threshold.

## Counting policy

| Input | Default |
| --- | --- |
| Reviewed pull request | Counted, whatever the ownership field says |
| Worker upload, `unchallenged` | Held (`worker_unchallenged`) |
| Worker upload, `exe_binding_v1` | Held (`worker_exe_binding_unverified`) |

`count_worker_unchallenged` and `count_worker_exe_binding` both default
to false. Setting one to true stops holding that class. It does not
verify a challenge response. The challenge endpoint does not exist yet.
This crate does not check `exe_binding_v1` bytes.

Held uploads are returned as object hash plus reason code, sorted by
hash then reason. They are not in `counted_uploads` and their hashes are
not tallied, so a held hash is neither accepted nor rejected.

A build mismatch is held as `build_id_mismatch` even when the upload
would also have been held for ownership. One upload is one held row.

## What these numbers do not prove

- They are not a count of people. Nothing in the manifest is an account.
- They are not proof of distinct PCs, GPUs, or drivers.
- A Worker ownership field is not evidence. Unchallenged means no server
  challenge. `exe_binding_v1` is stored unverified until a later check
  exists. That check is not this crate.
- Meeting the threshold is not a signed release and not a coverage index.
  Publishing is M3-B2 and is not done.
- Cross-GPU and cross-driver agreement of pipeline hashes is unproven.
  See [Open questions](#open-questions).

## Open questions

1. Cross-GPU and cross-driver agreement of pipeline hashes is
   **UNPROVEN**.
2. One real check so far: two separate sessions of the same game on the
   same PC and GPU gave 6416 of 6416 shared app keys, 6416 identical
   full records, and 2580 shared vertex/pixel shader pairs (session 2
   had 12 extra records). That is likely the easy case, because the game
   compiles most pipelines at load.
3. Games that compile pipelines lazily are untested.
4. There is no set of real uploads from 3 or more sources. Tests use
   synthetic manifests.

The [ARCHITECTURE.md](../ARCHITECTURE.md) sketch of `AgreementThreshold`
talks about description hashes plus bytecode hashes. This crate tallies
those lists and uses version 2 `pipeline_hashes` as the pipeline unit.
It still does not emit a `ConsensusIndex` or a coverage file.

## Safeguards

No bytecode and no hash keys of bytecode as publishable content. Opt-in
only. No accounts. No IPs. No game paths. Upload preview stays a client
concern. Offline extraction only. Never hook anti-cheat processes. No
Microsoft redistributables or trademarks as branding. No scraping or
reuse of any competitor database or of SCSKiller.
