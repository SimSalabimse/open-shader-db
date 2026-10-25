# open-shader-db

[![CI](https://github.com/SimSalabimse/open-shader-db/actions/workflows/ci.yml/badge.svg)](https://github.com/SimSalabimse/open-shader-db/actions/workflows/ci.yml)

A free, open, community-driven shader hash database and client for Direct3D 12 games using Microsoft's Advanced Shader Delivery (ASD) framework.

**Status:** Early design phase – not yet functional.

## What is open-shader-db?

open-shader-db is a community project that provides a free and open alternative to proprietary shader hash databases. It aims to help reduce shader compilation stuttering in Direct3D 12 games by distributing precompiled shader databases that work with Microsoft's Advanced Shader Delivery system.

This project consists of:

- **A community shader hash database** in Microsoft's SODB (State Object Database) format
- **A client tool** to download, merge, and apply shader databases locally
- **An upload and validation pipeline** to safely aggregate community contributions

## Why open-shader-db?

Shader compilation stutter is a significant problem in modern PC gaming. Microsoft's Advanced Shader Delivery (ASD) provides a technical solution, but requires games to ship SODB files. This project enables the community to build and share these databases for games that don't provide them officially.

**open-shader-db is:**
- **Free and open:** No paywalls, no accounts, no tracking
- **Privacy-first:** No IP logging, no game path collection, upload preview before sending
- **Community-driven:** Data comes only from opt-in contributors
- **Safe:** Anti-poisoning validation, sandboxed compilation, signed releases

## How is this different from SCSKiller?

[SCSKiller](https://github.com/BlueHeisenberg/SCSKiller) is an existing GPL-3.0-or-later shader compilation stutter mitigation tool. open-shader-db is a separate project with different design goals:

- **Freely redistributable:** Anyone can host and distribute the database
- **Community-run:** No single gatekeeper; transparent validation process
- **Privacy-first:** No accounts, no IP logging, upload preview before transmission
- **Open governance:** Community can audit, fork, and propose changes

Both projects aim to reduce shader stutter. We take different approaches to achieve that goal.

## Privacy promises

We are committed to protecting contributor privacy:

- **No user accounts or authentication required**
- **No IP address logging or storage**
- **Upload preview** before any data is transmitted
- **No game installation paths** are collected or transmitted
- **Only shader hashes and build identifiers** are shared – no game binaries, no personal data

All data collection is opt-in. You control what you share.

## How to contribute

**Currently:** This project is in early design. There is no working client or upload system yet.

**Soon:** Contributors will be able to:

1. Run the client tool to scan supported games
2. Preview the collected shader hashes locally
3. Upload anonymized data to the community database
4. Download the latest validated database for improved game performance

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on contributing code, documentation, or design feedback.

## Project status

This project is in **early design phase**. The local `osdb-cli` binary
already has these commands (synthetic / fixture tested; not a full
product):

- `inspect <file.sodb>` — print SODB hashes as JSON
- `discover` — list Steam games and SODB counts (JSON never includes paths)
- `coverage --sodb <file> --index <file>` — compare a local SODB to an
  unsigned coverage index (hashes and counts only)
- `records extract --sodb <file>` — write local pipeline-state records
  (hashes only, never bytecode; no network)
- `shaders scan --dir <path> | --sodb <file>` — hash local shaders
  (counts only; SODB or loose DXBC)
- `rebuild --records <file> (--dir|--sodb)... --out <file.sodb>` —
  rebuild a local SODB (contains bytecode; local use only;
  `--max-output-bytes` defaults to 8 GiB, hard maximum 8 GiB)
- `upload prepare --records <file> --build-id <id>` — build an
  anonymized upload manifest and print a preview of the exact
  bytes that would later be sent (nothing is sent; `--out` writes
  those bytes after `--yes-i-reviewed`)

The ingest Worker (`worker/`) and staging config are **built and
tested, not deployed**. See [docs/WORKER_DESIGN.md](docs/WORKER_DESIGN.md).

See [ROADMAP.md](ROADMAP.md) for planned milestones and [docs/FEATURE_MAP.md](docs/FEATURE_MAP.md) for current feature status.

## Hobby project and legal notice

This is a hobby project. Nothing in this repository is legal advice.

Contributions are opt-in and at the contributor's own risk. The project never redistributes shader bytecode.

open-shader-db is not affiliated with Microsoft, Valve, Epic, GOG, or any game publisher. Names of those companies, stores, and products are used only to describe compatibility.

See [docs/LEGAL_RISK_NOTES.md](docs/LEGAL_RISK_NOTES.md) for research notes (not legal advice; several points remain **UNVERIFIED**).

### Takedown requests

Rights holders who want a published record removed can open a [takedown request](https://github.com/SimSalabimse/open-shader-db/issues/new?template=takedown_request.yml). Do not attach bytecode or game files. The project will remove the record from future releases promptly and review the request in good faith.

## Important disclaimer

**This project is UNOFFICIAL and NOT AFFILIATED with [SCSKiller](https://github.com/BlueHeisenberg/SCSKiller), its developers, or any game publishers.**

We do not use, copy, or derive from SCSKiller's code or data. All data in this database comes exclusively from opt-in community contributors who choose to share information with this project.

## License

- **Code:** MIT License (see [LICENSE](LICENSE))
- **Data:** CC0 1.0 Universal (see [DATA-LICENSE.md](DATA-LICENSE.md))

See [LICENSE-PROPOSAL.md](LICENSE-PROPOSAL.md) for rationale.

## Learn more

- [ARCHITECTURE.md](ARCHITECTURE.md) – Technical design and data model
- [docs/LEGAL_RISK_NOTES.md](docs/LEGAL_RISK_NOTES.md) – Legal-risk research notes (not legal advice)
- [ROADMAP.md](ROADMAP.md) – Development milestones and planned features
- [CONTRIBUTING.md](CONTRIBUTING.md) – How to contribute
- [SECURITY.md](SECURITY.md) – Security policy and reporting

## References

- [Microsoft: Advanced Shader Delivery overview](https://learn.microsoft.com/en-us/windows/win32/direct3d12/advanced-shader-delivery)
- [Microsoft: State Object Database specification](https://microsoft.github.io/DirectX-Specs/d3d/StateObjectDatabase.html)
- [Microsoft: DirectX Graphics Samples - Advanced Shader Delivery](https://github.com/microsoft/DirectX-Graphics-Samples/tree/master/Samples/Desktop/D3D12AdvancedShaderDelivery)

---

**Maintainer:** Simsalabim  
**License:** Code is MIT ([LICENSE](LICENSE)); published data is CC0 1.0 Universal ([DATA-LICENSE.md](DATA-LICENSE.md))
