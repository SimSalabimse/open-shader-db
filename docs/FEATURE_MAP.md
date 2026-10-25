# Feature Map

This document tracks the implementation status of planned features. Milestone names match GitHub exactly.

**Last updated:** October 24, 2026

## Legend

- ✅ **Complete:** Implemented and tested
- ⚠️ **Partial:** Implemented with documented limitations
- 🚧 **In Progress:** Currently being developed
- 📋 **Planned:** Designed and scheduled
- 💭 **Proposed:** Under consideration or unscheduled
- ❌ **Blocked:** Waiting on external dependency

## Core features

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| Project documentation | ✅ Complete | Repository | M0 Index and coverage reports | README, ARCHITECTURE, ROADMAP, CONTRIBUTING |
| Licence files (MIT + CC0) | ✅ Complete | Repository | M0 Index and coverage reports | LICENSE and DATA-LICENSE.md |
| Contributing and security docs | ✅ Complete | Repository | M0 Index and coverage reports | CONTRIBUTING.md and SECURITY.md |

## Client tool

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| Game discovery (Steam) | ⚠️ Partial | crates/steam (osdb-steam) | M0 Index and coverage reports | Temp-dir fixtures: VDF, two libraries, SODB scan, missing root, hostile VDF, installdir confinement, symlink escape/loop/internal (Unix), walk bounds (depth, files, entries, time), path isolation. Untested: real Windows registry lookups and real Steam / game folders |
| SODB file parsing | ✅ Complete | crates/sodb | M0 Index and coverage reports | SQLite `user_version=2`; hashes only |
| Description hash generation | ⚠️ Partial | crates/sodb | M0 Index and coverage reports | SHA-256 of descriptors and followed tables; `string_associations` is not hashed (see crates/sodb/README.md) |
| Coverage comparison CLI | 📋 Planned | Client | M0 Index and coverage reports | Local SODB vs community index |
| Shader extraction and matching | 📋 Planned | Client | M1 Pipeline-state records | Needs legal clarity first |
| SODB rebuild from records + local shaders | 📋 Planned | Client | M1 Pipeline-state records | Needs legal clarity first |
| Upload manifest generation | 📋 Planned | Client | M2 Upload preview and submission | Anonymized JSON |
| Upload preview | 📋 Planned | Client | M2 Upload preview and submission | Show payload before send |
| Proof of ownership (client) | 📋 Planned | Client | M2 Upload preview and submission | Challenge-response with exe |
| Upload submission | 📋 Planned | Client | M2 Upload preview and submission | HTTPS POST to Worker |
| Index download | 📋 Planned | Client | M4 Signed releases | Fetch signed files |
| Signature verification | 📋 Planned | Client | M4 Signed releases | Minisign only |
| Game discovery (Epic, GOG) | 💭 Proposed | Client | Later / unscheduled | Not a milestone |
| GUI | 💭 Proposed | Client | Later / unscheduled | Not a milestone |
| Auto-update checking | 💭 Proposed | Client | Later / unscheduled | Not a milestone |

## Ingest pipeline

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| Ingest Worker (Cloudflare) | 📋 Planned | Ingest | M2 Upload preview and submission | HTTP POST |
| Schema validation | 📋 Planned | Ingest | M2 Upload preview and submission | UploadManifest JSON |
| Staging storage (R2 or reviewed PRs) | 📋 Planned | Ingest | M2 Upload preview and submission | Expire staging after 90 days |
| Rate limiting | 📋 Planned | Ingest | M2 Upload preview and submission | Cloudflare edge + Turnstile; no stored IPs |
| No IP logging | 📋 Planned | Ingest | M2 Upload preview and submission | Logpush without ClientIP |

## Validation and publishing

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| Staging data loader | 📋 Planned | Validator | M3 Consensus and publishing | Load manifests per build |
| Consensus builder | 📋 Planned | Validator | M3 Consensus and publishing | Configurable upload threshold |
| Proof-of-ownership checks (server) | 📋 Planned | Validator | M3 Consensus and publishing | Verify challenge |
| Anomaly detection | 📋 Planned | Validator | M3 Consensus and publishing | Flag suspicious patterns |
| Manual review queue | 📋 Planned | Validator | M3 Consensus and publishing | Human approval |
| CoverageIndex / record generation | 📋 Planned | Validator | M3 Consensus and publishing | Publish to staging for signing |
| Windows + GPU compile validation | 💭 Proposed | Validator | M5 Windows validation | Deferred |

## Distribution

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| Minisign signing | 📋 Planned | Publisher | M4 Signed releases | Offline project key |
| SHA256 checksums | 📋 Planned | Publisher | M4 Signed releases | |
| GitHub Releases upload | 📋 Planned | Publisher | M4 Signed releases | Primary distribution |
| Download catalog | 📋 Planned | Publisher | M4 Signed releases | JSON catalog |
| Cloudflare R2 mirror | 💭 Proposed | Publisher | Later / unscheduled | Optional |

## Data and formats

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| CoverageIndex schema | 📋 Planned | Data model | M0 Index and coverage reports | Designed in ARCHITECTURE.md |
| PipelineStateRecord schema | 📋 Planned | Data model | M1 Pipeline-state records | Designed; implementation after legal clarity |
| UploadManifest schema | 📋 Planned | Data model | M2 Upload preview and submission | Designed in ARCHITECTURE.md |
| BuildId schema | 📋 Planned | Data model | M0 Index and coverage reports | Designed in ARCHITECTURE.md |

## Security and privacy

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| No user accounts | 📋 Planned | Privacy | M0 Index and coverage reports | Design decision |
| Upload preview | 📋 Planned | Privacy | M2 Upload preview and submission | |
| No game paths collected | 📋 Planned | Privacy | M2 Upload preview and submission | |
| Multi-upload consensus | 📋 Planned | Security | M3 Consensus and publishing | |
| Signed releases (Minisign) | 📋 Planned | Security | M4 Signed releases | Approved |
| Compile validation | 💭 Proposed | Security | M5 Windows validation | Deferred |

## Documentation and operations

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| README, ARCHITECTURE, ROADMAP | ✅ Complete | Docs | M0 Index and coverage reports | In-tree project docs |
| CI/CD pipeline | ✅ Complete | Operations | M0 Index and coverage reports | Markdown/YAML plus Rust fmt/clippy/test (`--locked`) |
| Issue and PR templates | ✅ Complete | Docs | M0 Index and coverage reports | Bug, feature, question, and PR templates |
| User guide and FAQ | 💭 Proposed | Docs | Later / unscheduled | |

---

**Maintainer:** Simsalabim  
**Last updated:** October 24, 2026
