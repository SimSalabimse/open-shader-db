# Data-source red lines

These rules bound what this project may publish. They are a gate for later work, not a feature.

## Uploads are hashes only

Contributors upload only hash lists plus minimal metadata. An upload does not include shader bytecode, pipeline-state or descriptor fields, file paths, accounts, or IP addresses.

## Unreal Engine pipeline caches

No reader for Unreal Engine pipeline caches (`*.upipelinecache`) exists in this repository today.

Any future reader may only:

- Feed local hashes into the existing hash-only upload path.
- Read only unencrypted files.
- Be clean-room: written from observed file behaviour, with no Unreal Engine source code ported or copied.
- Never use or handle pak decryption or AES keys.

It must never publish descriptor fields or per-game PSO tables derived from those caches.

## Descriptor-field records

Publishing descriptor-field records is allowed only for data that came from SODB files (Microsoft's own sharing format).
