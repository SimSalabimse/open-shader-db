# Feature Map

This document tracks the implementation status of planned features. Milestone names match GitHub exactly.

**Last updated:** October 26, 2026

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
| Game discovery (Steam) | ⚠️ Partial | crates/steam (osdb-steam) | M0 Index and coverage reports | Temp-dir fixtures: VDF, two libraries, SODB scan, missing root, hostile VDF, installdir confinement (no symlinked install dirs), symlink escape/loop/internal/race (Unix), walk bounds (depth, files, entries, time per game; the bound is named), breadth-first order, `.sodb` extension before open, stale library count, `--sodb-dir` (including `Audio` and a missing or non-directory path), explicit `--xbox-games` folders (including `Content`; no drive-letter probe), path isolation. Steam installs are not expected to contain `.sodb` files today (inferred, not verified; see docs/SODB_SOURCES.md). One explicit `--sodb-dir` capture folder reported `sodb_files=1` (tested once, works; see docs/CAPTURE_SODB.md). That does not show every game or every machine. Xbox app on-disk layout is **UNVERIFIED**. Untested: real Windows registry lookups |
| SODB file parsing | ✅ Complete | crates/sodb | M0 Index and coverage reports | SQLite `user_version=2`; hashes only |
| Description hash generation | ⚠️ Partial | crates/sodb | M0 Index and coverage reports | SHA-256 of descriptors and followed tables; `string_associations` is not hashed (see crates/sodb/README.md) |
| Coverage comparison CLI | ⚠️ Partial | crates/coverage, crates/osdb-cli | M0 Index and coverage reports | `osdb-cli coverage` vs unsigned local index; synthetic tests only. Coverage of the one user-captured SODB was not run. **Untested:** coverage of a real game SODB, real community indexes, signed indexes (M4), network download. Consensus and cross-machine agreement are not shown |
| Shader extraction and matching | ⚠️ Partial | crates/shaders (osdb-shaders), crates/records, crates/osdb-cli | M1 Pipeline-state records | SODB scan + loose DXBC directory scan + `match_records`. No network, no process attach. Legal review done (see docs/LEGAL_RISK_NOTES.md). One user-captured SODB: `records extract` tested once, works (see docs/CAPTURE_SODB.md). **UNVERIFIED:** other games, `shaders scan` and hash matching on a real capture, packed archives |
| SODB rebuild from records + local shaders | ⚠️ Partial | crates/rebuild (osdb-rebuild), crates/osdb-cli | M1 Pipeline-state records | Local rebuild from records + scanned shaders. Synthetic round-trip tested. Rebuild of the one captured SODB was not run. **UNVERIFIED:** real Microsoft runtime acceptance, rebuild of a real game SODB |
| Upload manifest generation | ⚠️ Partial | crates/upload (osdb-upload), crates/osdb-cli | M2 Upload preview and submission | Local `upload prepare` only. Schema in schemas/upload_manifest.v1.json. No network. Description hashes are record descriptor keys, not SODB CanonicalHasher hashes (UNVERIFIED for consensus) |
| Upload preview | ⚠️ Partial | crates/upload, crates/osdb-cli | M2 Upload preview and submission | CLI preview from the same canonical bytes as `--out`. No GUI. One real preview was printed and sent nothing (tested once, works; see docs/CAPTURE_SODB.md). That is not consensus |
| Proof of ownership (client) | ⚠️ Partial | crates/upload | M2 Upload preview and submission | `OwnershipProof::Unchallenged` only. `exe_binding_v1` is in the schema and is UNVERIFIED |
| Upload submission | 📋 Planned | Client | M2 Upload preview and submission | HTTPS POST to Worker |
| Index download | 📋 Planned | Client | M4 Signed releases | Fetch signed files |
| Signature verification | 📋 Planned | Client | M4 Signed releases | Minisign only |
| Game discovery (Epic, GOG) | 💭 Proposed | Client | Later / unscheduled | Not a milestone |
| GUI | 💭 Proposed | Client | Later / unscheduled | Not a milestone |
| Auto-update checking | 💭 Proposed | Client | Later / unscheduled | Not a milestone |

## Ingest pipeline

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| Ingest Worker (Cloudflare) | ⚠️ Partial | worker/ | M2 Upload preview and submission | `POST /v1/upload` built and tested. **Not deployed.** Placeholders only |
| Schema validation | ⚠️ Partial | crates/upload, worker/, schemas/ | M2 Upload preview and submission | Shared golden vectors. JSON Schema is looser than the parsers. Worker does not use the schema alone |
| Staging storage (R2 or reviewed PRs) | ⚠️ Partial | worker/, worker/config/ | M2 Upload preview and submission | Key layout, 90-day lifecycle file, access policy in WORKER_DESIGN. **Not deployed.** Reviewed-PR path documented |
| Rate limiting | ⚠️ Partial | worker/ | M2 Upload preview and submission | Constant-key wrangler binding (`v1-upload`), approximate and per location. One client can consume the shared budget. No IP counters in our storage. **Not deployed** |
| No IP logging | ⚠️ Partial | worker/, worker/config/ | M2 Upload preview and submission | Source lint + Logpush exact allowlist (no `Event`). Cloudflare edge logs remain outside our control. **Not deployed** |

## Validation and publishing

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| Staging data loader | ⚠️ Partial | crates/consensus (osdb-consensus) | M3 Consensus and publishing | `load_build` / `list_builds`, also used by `osdb-cli consensus`. Read-only input tree, hashes and skip counts. Not deployed. See docs/CONSENSUS.md |
| Consensus builder | ⚠️ Partial | crates/consensus (osdb-consensus) | M3 Consensus and publishing | Threshold default 3 uploads and 500 per mille; not lowered for a small build. Local CLI `consensus build` / `report`. Unsigned files only. Not deployed. Counts do not prove independent uploaders. Cross-GPU pipeline agreement is UNPROVEN. See docs/CONSENSUS.md |
| Proof-of-ownership checks (server) | 📋 Planned | Validator | M3 Consensus and publishing | Not implemented. The consensus library holds Worker `unchallenged` and `exe_binding_v1` uploads by default because the challenge endpoint does not exist |
| Anomaly detection | 📋 Planned | Validator | M3 Consensus and publishing | Not started. M3-C is not done |
| Manual review queue | 📋 Planned | Validator | M3 Consensus and publishing | Not started. Reviewed pull requests are an ingest path the loader can be pointed at; there is no queue |
| CoverageIndex / record generation | ⚠️ Partial | crates/consensus, crates/coverage, crates/osdb-cli | M3 Consensus and publishing | Local unsigned staging only (`coverage.v2.json`, count report, optional records). Tooling exists. Nothing deployed. No real data published. M3-C and signing are not done |
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
| CoverageIndex schema | ⚠️ Partial | crates/coverage | M0 Index and coverage reports | format_version 1 (SODB description hashes) and format_version 2 (pipeline content hashes) in docs/COVERAGE_INDEX_FORMAT.md. Logical DescriptionHash objects are still not stored. v2 is unsigned local staging; no index is published |
| PipelineStateRecord schema | ⚠️ Partial | crates/records | M1 Pipeline-state records | format_version 1 JSON in docs/RECORD_FORMAT.md (local files). Legal review done (see docs/LEGAL_RISK_NOTES.md). State objects not included. One user-captured SODB produced a records file (tested once, works; see docs/CAPTURE_SODB.md). A second session of the same game on the same PC and GPU matched 6416 of 6416 app keys and 6416 identical full records (session 2 had 12 extra records; see docs/CONSENSUS.md). **Untested:** rebuild fidelity, non-SODB engines, lazy pipeline compile, cross-GPU and cross-driver agreement |
| UploadManifest schema | ⚠️ Partial | crates/upload | M2 Upload preview and submission | schema_version 1 and 2 in docs/UPLOAD_MANIFEST.md, schemas/upload_manifest.v1.json, and schemas/upload_manifest.v2.json. Version 2 adds pipeline_hashes. Drift-tested against Rust types |
| BuildId schema | ⚠️ Partial | crates/coverage | M0 Index and coverage reports | On-disk game_id / version / exe_hash on the index. SODB has no build identifier yet; coverage reports `build_identity: not_compared` |

## Security and privacy

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| No user accounts | 📋 Planned | Privacy | M0 Index and coverage reports | Design decision |
| Upload preview | ⚠️ Partial | crates/upload, crates/osdb-cli | M2 Upload preview and submission | Local preview; canary tests. No send |
| No game paths collected | ⚠️ Partial | crates/upload, crates/osdb-cli, worker/ | M2 Upload preview and submission | Manifest, preview, errors, Debug, stored object, and Worker receipt omit paths (tested). Worker not deployed |
| Multi-upload consensus | ⚠️ Partial | crates/consensus (osdb-consensus) | M3 Consensus and publishing | Local threshold and unsigned publish staging. No accounts, so counts are uploads, not people, and do not prove uploader independence. One same-PC same-GPU check: 6416 of 6416 shared app keys, 6416 identical full records, 2580 shared vertex/pixel shader pairs (session 2 had 12 extra records). Cross-GPU and cross-driver pipeline agreement is UNPROVEN. Lazy pipeline compile is untested. No real 3+ uploads; tests are synthetic. Nothing deployed. M3-C and signing are not done. See docs/CONSENSUS.md |
| Signed releases (Minisign) | 📋 Planned | Security | M4 Signed releases | Approved |
| Compile validation | 💭 Proposed | Security | M5 Windows validation | Deferred |

## Documentation and operations

| Feature | Status | Owner module | Milestone | Notes |
|---------|--------|--------------|-----------|-------|
| README, ARCHITECTURE, ROADMAP | ✅ Complete | Docs | M0 Index and coverage reports | In-tree project docs |
| CI/CD pipeline | ✅ Complete | Operations | M0 Index and coverage reports | Markdown/YAML plus Rust fmt/clippy/test (`--locked`) |
| Issue and PR templates | ✅ Complete | Docs | M0 Index and coverage reports | Bug, feature, question, takedown, and PR templates |
| User guide and FAQ | 💭 Proposed | Docs | Later / unscheduled | |

---

**Maintainer:** Simsalabim  
**Last updated:** October 26, 2026
