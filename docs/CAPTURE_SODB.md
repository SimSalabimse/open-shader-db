# Capture an SODB on your own PC

This page is optional. It is for a tester or developer who wants a State
Object Database (SODB) of a game they already own, on the PC where that
game is installed, and then wants to run `osdb-cli` on that file.
`osdb-cli` does not record a play session and does not attach to a
process. A runtime recorder inside this project (issue #32) is design
only.

Capture stays on your PC. Do not share the `.sodb` file. Do not upload
it. Do not attach this project's tools, a debugger, or anything else to
a running anti-cheat game.

## What an SODB is

An SODB is a developer artifact: the serialized pipeline-state and
state-object descriptions from a session, together with the shader DXIL
needed to reconstruct them. That file can contain shader bytecode. The
`.sodb` file itself is never shared and never uploaded. This project
reads a file you already have. It does not send it anywhere.

Microsoft's Advanced Shader Delivery pipeline is separate: a developer
creates an SODB, can compile it locally to a PSDB to test, and a
storefront with that integration delivers a PSDB to players. See
[Where do SODB files come from?](SODB_SOURCES.md). This guide does not
ask you to send an SODB to a store.

Sources:

- [Advanced Shader Delivery overview](https://learn.microsoft.com/en-us/windows/win32/direct3d12/advanced-shader-delivery) (Microsoft Learn)
- [D3D12 State Object Database sample](https://learn.microsoft.com/en-us/samples/microsoft/directx-graphics-samples/d3d12-state-object-database-sample-win32/) (Microsoft Learn)
- [DirectX-Graphics-Samples Advanced Shader Delivery readme](https://github.com/microsoft/DirectX-Graphics-Samples/blob/master/Samples/Desktop/D3D12AdvancedShaderDelivery/readme.md)

## Which games

Use one single-player, offline game that has no anti-cheat. If you are
not sure, skip that title. Do not use a live-service or protected
online title. Do not point `d3dconfig` at an anti-cheat game.

The capture tool is Microsoft's `d3dconfig`, not a hook from this
project. The Learn sample says the runtime records pipeline state
objects and state objects created by the D3D12 API, and that no
application code changes are required for that method. Settings are
read at device creation, so exit the game before you change them and
start the game again after.

## Capture with d3dconfig

The samples readme says `d3dconfig.exe` and
`D3D12StateObjectCompiler.exe` ship in the Agility SDK (1.619.5 or
newer) and names Windows 11, version 24H2 (build 26100.1000) or newer.
**UNVERIFIED:** the directory those executables are installed into. No
page linked above gives that path.

The Learn sample gives these commands. The add command uses two dashes.
`<exefilename>` may be a full path; the same page notes that a hard
link may not match. Every matching app writes to the database you set.

```
d3dconfig.exe apps --add <exefilename>
d3dconfig.exe device pso-db-path=<path-to-sodb>
d3dconfig.exe device enable-pso-db=true
```

Play, then exit. The Learn sample says the file written is the path
from `pso-db-path`. These sources do not name a default folder.
**UNVERIFIED:** any landing place other than the path you set.

When you are done, clear the settings (Learn sample):

```
d3dconfig.exe --reset
```

The samples readme adds a checkpoint that folds a write-ahead log back
into the SODB. The Learn sample page does not mention this command:

```
D3D12StateObjectCompiler.exe checkpoint-db <path-to-sodb>
```

**UNVERIFIED:** whether `osdb-cli` can open a capture before that
checkpoint. Run the checkpoint, then use the same `<path-to-sodb>`.

## Run osdb-cli

Quote every path. Placeholders only: no account name, no real install
path. `<user>` stands in for the test folder you choose.

Count the file without printing its path in JSON:

```
osdb-cli discover --sodb-dir "<path-to-folder>"
```

Human output includes a line `explicit SODB folder  sodb_files=N`.
`--json` does not include paths.

Hash the game executable yourself (64 lowercase hex digits). Do not
send that path:

```
osdb-cli records extract --sodb "<path-to-sodb>" --out "<user>\osdb-test\records.json" --game-id title1 --version 1 --exe-hash <64 lowercase hex>
```

`--game-id` and `--version` are 1–128 graphic ASCII characters with no
space, `/`, `\`, or `:`. Do not use the store display name. A quiet
success prints nothing. A skipped root signature is a count on stderr.
Do not send `records.json`.

Preview the upload. Nothing is sent:

```
osdb-cli upload prepare --records "<user>\osdb-test\records.json" --build-id title1
```

The preview starts with `Upload preview (N bytes)`, then `These exact
bytes would be sent by a later upload step.` and `Nothing is sent by
this command. There is no network upload.` If `--exe-hash` was omitted,
it also prints `build identity is incomplete`.

## What to send back

Send counts and hashes only: the `sodb_files` count, any skipped
root-signature count, the preview byte count, and the hashes printed
under `Canonical JSON`. Do not send the `.sodb`, shader bytes,
`records.json`, a path, or a username. If a line contains a path or an
account name, delete that line. The longer checklist is in
[TESTING_WINDOWS.md](TESTING_WINDOWS.md).
