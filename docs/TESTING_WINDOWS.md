# Try the Windows CLI on one installed game

This is a short check for issue #9: run `osdb-cli` on **one** real game
install. Close all games before running discover (it reads every
installed game's folder, read-only). The tool never attaches to a game
process. **This build has no network code.** Nothing is uploaded. Reads
game files; writes only the `--out` files you name in your test folder.

The binary is **unsigned**. That is expected.

## Get the program

A maintainer builds it from the Actions tab: open **Windows CLI**, then
**Run workflow**. Use only the run link the maintainer sends you, and
compare the hash to the one in the maintainer's message
(`SHA256SUMS.txt` in the zip only catches a broken download).

1. From that run, download `osdb-cli-windows-x86_64`. Unzip it.
2. You should have `osdb-cli.exe`, `SHA256SUMS.txt`, and this guide.
3. In PowerShell, in that folder:

```
(Get-FileHash .\osdb-cli.exe -Algorithm SHA256).Hash
```

The 64 hex digits must match the hash in the maintainer's message
(ignore letter case). If they do not, stop.

If Windows blocks the file, run `Unblock-File .\osdb-cli.exe`. If it is
still blocked, stop and report `blocked`; do not turn off any security
feature. Do not send the `.exe` back.

Make the test folder once. Always wrap every path in double quotes
(discover prints paths with spaces and `(x86)`):

```
mkdir $env:USERPROFILE\osdb-test
```

Never write into the game folder.

## Pick one game

Use **one** title you already installed through Steam that has **no
anti-cheat**. If you are not sure, skip that title. Single-player,
offline titles are the intended target. Do not pick a live-service or
protected online title.

If your game shows `sodb_files=0`, stop and send that count. Packed
game archives are not scanned.

Real Steam folders, Windows registry Steam lookup, and real game SODBs
are **UNVERIFIED**. Coverage against a real index: **UNVERIFIED** (no
index exists yet). Whether a runtime accepts a rebuilt SODB, and
shaders inside packed archives, are **UNVERIFIED**.

## Commands (in this order)

Use PowerShell. `cd` to the unzipped folder. Quote every path.

**1. Discover** (omit `--steam-root` unless Steam is not in the usual
place):

```
.\osdb-cli.exe discover --show-paths
```

Human lines look like:

```
NameFromSteam  app=123  build=456  sodb_files=1
  install: <path on your PC; do not send>
  sodb: <path on your PC; do not send>
```

If nothing is installed where the tool looks:

```
No games found.
```

and stderr may say `warning: no Steam install directory was found` or
`warning: Steam root does not exist or is not a directory`.

JSON (`--json`) never includes paths. Real names, app ids, and counts
from a live library are **UNVERIFIED**.

**2. Coverage** needs a local unsigned index JSON. There is no published
community index yet. If you do not have one, skip this step and write
`coverage: skipped (no index)`. Coverage against a real index:
**UNVERIFIED**. If you do have an index:

```
.\osdb-cli.exe coverage --sodb "C:\Program Files (x86)\Steam\steamapps\common\Game\x.sodb" --index "$env:USERPROFILE\osdb-test\index.json"
```

Printed shape (counts vary). The last three lines are always these
exact strings. `(none)` and `0 unmatched hashes omitted` change when
the index and the SODB do not share every hash (unmatched hashes are
listed; `1 unmatched hash omitted` if exactly one is hidden):

```
Coverage report
Local description hashes: N
Index description hashes: N
Matched: N
Unmatched: N
Index hashes absent locally: N
Match rate: A/B (P%)
Unmatched description hashes:
(none)
0 unmatched hashes omitted
Build identity: not_compared (SODB carries no build identifier yet)
Index status: unsigned index, not verified
Index: unsigned (signature verification arrives in a later milestone)
```

**3. Extract records** (hashes only). Get the hash of the game's main
`.exe` (in its install folder). The CLI needs 64 **lowercase** hex;
uppercase fails with `malformed hex in record file`:

```
(Get-FileHash "C:\Program Files (x86)\Steam\steamapps\common\Game\game.exe" -Algorithm SHA256).Hash.ToLower()
```

Paste that as `--exe-hash`. If you leave it out, step 6 prints
`build identity is incomplete`.

```
.\osdb-cli.exe records extract --sodb "C:\Program Files (x86)\Steam\steamapps\common\Game\x.sodb" --out "$env:USERPROFILE\osdb-test\records.json" --game-id title1 --version 1 --exe-hash <paste lowercase hash>
```

`--game-id` / `--version` must be 1–128 graphic ASCII with no space,
`/`, `\`, or `:`. Do not use the Steam display name. A quiet success
prints nothing. If a root-signature blob was skipped, stderr is
`skipped N root signature` or `skipped N root signatures`. Do not send
`records.json`.

**4. Scan local shaders** (counts only):

```
.\osdb-cli.exe shaders scan --sodb "C:\Program Files (x86)\Steam\steamapps\common\Game\x.sodb"
```

Exact line shape (numbers **UNVERIFIED** on a real game):

```
files_seen=N shaders_indexed=N unique_hashes=N duplicates=N
skipped: symlink=N special_file=N too_large=N corrupt=N unreadable=N
corrupt: empty=N truncated=N size_exceeds_file=N
```

(`shaders scan --dir` is for a maintainer who already has loose DXBC
files. Do not use both `--dir` and `--sodb`.)

**5. Rebuild** (local file with bytecode; **do not share it**):

```
.\osdb-cli.exe rebuild --records "$env:USERPROFILE\osdb-test\records.json" --sodb "C:\Program Files (x86)\Steam\steamapps\common\Game\x.sodb" --out "$env:USERPROFILE\osdb-test\rebuilt.sodb"
```

Stderr (exact first line):

```
Rebuilt SODB is for local use only and must not be shared or uploaded.
rebuilt=N skipped_missing_stage=N skipped_unresolved_stages=N skipped_root_signature=N skipped_noncanonical_root_signature=N unique_shaders=N unique_root_signatures=N publish_warnings=N
```

Runtime acceptance of that file is **UNVERIFIED**.

**6. Upload preview** (still offline; nothing is sent):

```
.\osdb-cli.exe upload prepare --records "$env:USERPROFILE\osdb-test\records.json" --build-id title1
```

If step 3 stored version and exe hash, you can omit those flags here.
If `--exe-hash` was left out, this command prints
`build identity is incomplete`. The preview always starts with:

```
Upload preview (N bytes)
These exact bytes would be sent by a later upload step.
Nothing is sent by this command. There is no network upload.
```

then `Fields that will be sent:` (array lengths, not the hash lists in
that block), then
`Ownership is unchallenged (no_server_challenge). This field proves nothing. UNVERIFIED until a server challenge exists.`,
then `Canonical JSON:` and the compact JSON. To write that JSON locally
(optional): add
`--out "$env:USERPROFILE\osdb-test\preview.json" --yes-i-reviewed`
only after you have read the preview.

If the canonical manifest is larger than the 8 MiB upload wire cap
(8388608 bytes), the command prints an error and writes nothing. The
error names both sizes and says this manifest would be rejected by the
upload server; nothing was written. It does not print the preview and
it does not include a path. That cap is the same limit the upload
server enforces. For a local copy of an over-cap manifest, add
`--allow-oversize`. Stderr then says the manifest is over the wire
limit and nothing was sent. The server would still reject the file.

## Success

- The `.exe` hash matches the maintainer's message.
- Discover ran. If `sodb_files=0`, you stopped and sent that count.
- Later steps either printed the count lines above or a short error
  with **no** file path in it.
- Game files were not modified. You did not send shader bytes, a
  rebuilt `.sodb`, or anything from the game folder.

## If something fails

- **Discover empty:** Steam may not be in a default location. Try
  `.\osdb-cli.exe discover --steam-root "C:\Program Files (x86)\Steam" --show-paths`.
  Windows registry lookup is compiled in but **UNVERIFIED**.
- **`sodb_files=0`:** stop and send that count.
- **Coverage error:** missing index is expected. Skip it.
- **Extract / scan / rebuild / upload error:** copy the one-line
  message only if it has no path and no username.
  `output file already exists (pass --force to overwrite)` means pick
  a new `--out` name. `records extract`, `shaders scan`, and
  `upload prepare` print `output path is the same file as the input`.
  `rebuild` prints `rebuild: output path is the same file as an input`.
  A missing test folder can print `I/O error` or
  `rebuild: output parent directory is missing` — run
  `mkdir $env:USERPROFILE\osdb-test` first.
- **Do not retry on an anti-cheat title.**

## What to send back

Send **only** counts and hashes from the printed summaries and from the
upload preview JSON. Never send shader bytes, never any file from the
game folder, never the rebuilt `.sodb`, never `records.json`, never a
game path, never a username. Read the text before you send it. If you
see a path or account name, delete that line.

Suggested paste:

```
osdb-cli --version:
discover: games=  sodb_file_count=  (no names, no paths)
coverage: skipped or Match rate A/B (P%) matched= unmatched=
records extract: ok / error text; skipped root signatures=
shaders scan: files_seen= shaders_indexed= unique_hashes= duplicates= skipped_corrupt=
rebuild: rebuilt= unique_shaders= skipped_*=
upload preview: byte count=  description_hashes (N)=  bytecode_hashes (N)=
  schema_version=  client_version=  collected_hour=  ownership.kind=
  (hashes from Canonical JSON only; no paths)
UNVERIFIED notes:
```
