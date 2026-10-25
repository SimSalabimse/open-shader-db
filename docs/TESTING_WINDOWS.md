# Try the Windows CLI on one installed game

This is a short check for issue #9: run `osdb-cli` on **one** real game
install. The game must stay closed. The tool only reads files on disk. It
never attaches to a game process. **This build has no network code.**
Nothing is uploaded. `rebuild` and `upload prepare --out` write only the
local files you name.

The binary is **unsigned**. That is expected.

## Get the program

A maintainer builds it from the Actions tab: open **Windows CLI**, then
**Run workflow**. Testers download the artifact from that run (or from
the pull request that added this workflow).

1. Open the Actions tab, open a **Windows CLI** run, download
   `osdb-cli-windows-x86_64`. Unzip it.
2. You should have `osdb-cli.exe`, `SHA256SUMS.txt`, and this guide.
3. In PowerShell, in that folder:

```
Get-FileHash .\osdb-cli.exe -Algorithm SHA256
Get-Content .\SHA256SUMS.txt
```

The 64 hex digits must match (ignore letter case). If they do not, stop.

The system may warn that the file is unsigned. Do not send the `.exe`
back. Keep work files in a new folder such as `%USERPROFILE%\osdb-test`.
Never write into the game folder.

## Pick one game

Use **one** title you already installed through Steam that has **no
anti-cheat**. If you are not sure, skip that title. Single-player, offline
titles are the intended target. Do not pick a live-service or protected
online title.

Leave the game closed. This project never touches a running game.

Real Steam folders and real game SODBs are **UNVERIFIED**. Many installs
have zero `.sodb` files. That is a useful result. Stop after discover if
you have no SODB and no loose files that start with `DXBC`. Packed game
archives are not scanned.

## Commands (in this order)

In PowerShell, `cd` to the unzipped folder. Replace `<sodb>` with a path
`discover` printed for you. Replace `<work>` with your test folder.
Do **not** paste those paths in what you send back.

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
`coverage: skipped (no index)`. If you do:

```
.\osdb-cli.exe coverage --sodb <sodb> --index <index.json>
```

The printed shape (counts vary; last two lines are exact):

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

**3. Extract records** (hashes only; writes `<work>\records.json`):

```
.\osdb-cli.exe records extract --sodb <sodb> --out <work>\records.json --game-id title1 --version 1 --exe-hash <64 lowercase hex of the game exe>
```

`--game-id` / `--version` must be 1–128 graphic ASCII with no space, `/`,
`\`, or `:`. Do not use the Steam display name. A quiet success prints
nothing. If a root-signature blob was skipped, stderr is
`skipped N root signature` or `skipped N root signatures`. Do not send
`records.json`.

**4. Scan local shaders** (counts only):

```
.\osdb-cli.exe shaders scan --sodb <sodb>
```

Exact line shape (numbers **UNVERIFIED** on a real game):

```
files_seen=N shaders_indexed=N unique_hashes=N duplicates=N
skipped: symlink=N special_file=N too_large=N corrupt=N unreadable=N
corrupt: empty=N truncated=N size_exceeds_file=N
```

Loose DXBC: `shaders scan --dir <folder>` instead. Not both flags.

**5. Rebuild** (local file with bytecode; **do not share it**):

```
.\osdb-cli.exe rebuild --records <work>\records.json --sodb <sodb> --out <work>\rebuilt.sodb
```

Stderr (exact first line):

```
Rebuilt SODB is for local use only and must not be shared or uploaded.
rebuilt=N skipped_missing_stage=N skipped_unresolved_stages=N skipped_root_signature=N skipped_noncanonical_root_signature=N unique_shaders=N unique_root_signatures=N publish_warnings=N
```

Whether a game or runtime accepts that file is **UNVERIFIED**.

**6. Upload preview** (still offline; nothing is sent):

```
.\osdb-cli.exe upload prepare --records <work>\records.json --build-id title1
```

If extract already stored version and exe hash, you can omit those
flags. If it prints `build identity is incomplete`, pass `--version` and
`--exe-hash`. The preview always starts with:

```
Upload preview (N bytes)
These exact bytes would be sent by a later upload step.
Nothing is sent by this command. There is no network upload.
```

then `Fields that will be sent:` (array lengths, not the hash lists in
that block), then
`Ownership is unchallenged (no_server_challenge). This field proves nothing. UNVERIFIED until a server challenge exists.`,
then `Canonical JSON:` and the compact JSON. To write that JSON locally
(optional): add `--out <work>\preview.json --yes-i-reviewed` only after
you have read the preview.

## Success

- The `.exe` hash matches `SHA256SUMS.txt`.
- Discover ran. You either have one game with `sodb_files` ≥ 1, or you
  stopped with a count of zero SODBs.
- Later steps either printed the count lines above or a short error with
  **no** file path in it.
- Game files were not modified. You did not send shader bytes, a rebuilt
  `.sodb`, or anything from the game folder.

## If something fails

- **Discover empty:** Steam may not be in a default location. Try
  `.\osdb-cli.exe discover --steam-root <Steam folder> --show-paths`.
  Windows registry lookup is compiled in but **UNVERIFIED**.
- **Games listed, `sodb_files=0`:** stop. Send only the count. Do not
  hunt inside archives.
- **Coverage error:** missing index is expected. Skip it.
- **Extract / scan / rebuild error:** copy the one-line message only if
  it has no path and no username. `output file already exists (pass --force to overwrite)`
  means pick a new `--out` name. `output path is the same file as the input SODB`
  means do not write over the game file.
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
