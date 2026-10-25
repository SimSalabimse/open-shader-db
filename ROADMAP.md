# Roadmap

This document outlines the development milestones and planned features for open-shader-db. GitHub milestones use these names exactly. Each milestone has associated issues drafted in [docs/issues-draft.md](docs/issues-draft.md).

**Last updated:** October 24, 2026  
**Status:** Early design phase

## Milestone overview

| Milestone | Status | Target | Description |
|-----------|--------|--------|-------------|
| [M0 Index and coverage reports](#m0-index-and-coverage-reports) | 📋 Planned | First ship | Foundations (docs, licence, CI, templates) plus index-only verification |
| [M1 Pipeline-state records](#m1-pipeline-state-records) | 📋 Planned | After legal clarity | State records without bytecode and client rebuild |
| [M2 Upload preview and submission](#m2-upload-preview-and-submission) | 📋 Planned | After M0 client | Client upload preview/submit plus Cloudflare ingest Worker |
| [M3 Consensus and publishing](#m3-consensus-and-publishing) | 📋 Planned | After ingest | Consensus, proof-of-ownership checks, review queue, publishing |
| [M4 Signed releases](#m4-signed-releases) | 📋 Planned | After consensus | Minisign signing, GitHub Releases, client verification |
| [M5 Windows validation](#m5-windows-validation) | 💭 Deferred | Later | Windows + GPU compile validation |

Community GUI, Epic/GOG discovery, and similar items are [later / unscheduled](#later--unscheduled), not a milestone.

---

## M0 Index and coverage reports

**Goal:** Ship foundations and index-only verification so a user can compare a local SODB against a community coverage index.

**Status:** 📋 Planned  
**Ships:** Yes (first release)

### Deliverables

- [x] Documentation (README, ARCHITECTURE, ROADMAP, and related files)
- [x] Licence files (MIT for code, CC0 for published data)
- [x] CI/CD pipeline (GitHub Actions)
- [x] Issue and PR templates
- [x] Language choice for the client
- [x] SODB SQLite parser
- [ ] Steam game discovery (partial; temp-dir fixtures — see FEATURE_MAP)
- [ ] Coverage comparison CLI (local SODB vs community index)

### Success criteria

- [ ] Documentation is complete and merged
- [ ] LICENSE and DATA-LICENSE.md are in the repository
- [ ] CI validates documentation
- [ ] Client can parse a local SODB and report coverage against a community index
- [ ] First coverage index published for at least one game

---

## M1 Pipeline-state records

**Goal:** Publish pipeline-state records without DXIL bytecode, and rebuild a usable SODB on the user's machine from those records plus shaders from the installed game.

**Status:** 📋 Planned  
**Ships:** After legal clarity

**Legal review is required before implementation.** See [ARCHITECTURE.md](ARCHITECTURE.md#legal-and-tos-doubts).

### Deliverables

- [ ] Legal review or documented risk acceptance
- [ ] Shader bytecode extraction and hashing from installed game files
- [ ] Pipeline-state record schema (state, root signature, formats; no DXIL)
- [ ] Client rebuild: published records + local shaders
- [ ] Write a rebuilt SODB and test it with a game

### Success criteria

- [ ] Client extracts DXIL from at least one installed game without modifying game files
- [ ] Client matches bytecode hashes to published records
- [ ] Client rebuilds a valid SODB locally
- [ ] Legal doubts in ARCHITECTURE.md are addressed or explicitly accepted

---

## M2 Upload preview and submission

**Goal:** Let a contributor preview an anonymized upload and submit it; receive it in a Cloudflare Worker and store it in staging.

**Status:** 📋 Planned

### Deliverables

- [ ] Upload manifest generation (rounded timestamp, no game paths)
- [ ] Upload preview UI
- [ ] Proof-of-ownership challenge on the client
- [ ] Cloudflare Worker ingest endpoint
- [ ] Staging storage (Cloudflare R2 or reviewed PRs)
- [ ] Cloudflare edge rate limiting + Turnstile (no stored IPs)

### Success criteria

- [ ] User can preview the exact payload before it is sent
- [ ] Worker accepts valid manifests and rejects invalid ones with clear errors
- [ ] No IP addresses are stored; Logpush omits ClientIP
- [ ] Rate limiting is edge-only (Cloudflare rules + Turnstile) plus a per-build upload cap

---

## M3 Consensus and publishing

**Goal:** Turn staged uploads into a published coverage index or record set using an upload threshold, proof-of-ownership checks, and a review queue.

**Status:** 📋 Planned

### Deliverables

- [ ] Staging data loader
- [ ] Consensus builder (configurable upload threshold)
- [ ] Server-side proof-of-ownership checks
- [ ] Anomaly detection and manual review queue
- [ ] Generation of CoverageIndex (M0) and PipelineStateRecord sets (M1)
- [ ] Publishing to a staging area for M4 to sign

### Success criteria

- [ ] Pipeline loads all staging manifests for a build
- [ ] Consensus uses a documented upload threshold (no weakening for low-population games)
- [ ] Anomalies are queued for human review
- [ ] Validated data is published to staging, not as unsigned production releases

---

## M4 Signed releases

**Goal:** Sign published data with Minisign (offline project key) and let the client verify before use.

**Status:** 📋 Planned

### Deliverables

- [ ] Minisign signing with an offline project key
- [ ] SHA256 checksums
- [ ] GitHub Releases (or R2) upload automation
- [ ] Download catalog JSON
- [ ] Client Minisign verification

### Success criteria

- [ ] Every release file has a `.minisig` and `.sha256`
- [ ] Client rejects invalid signatures
- [ ] Public key is published; rotation is documented

---

## M5 Windows validation

**Goal:** Optionally compile SODB to PSDB on Windows with a GPU and vendor compiler to catch bad records before publish.

**Status:** 💭 Deferred (later)

M0 through M4 do not depend on this. They rely on proof of ownership, an upload threshold, and reviewed PRs.

### Deliverables (when started)

- [ ] Windows sandbox or VM with GPU access
- [ ] Automated SODB to PSDB compilation
- [ ] Integration with the M3 consensus pipeline

### Success criteria (when implemented)

- [ ] Compile failures are caught before a signed release
- [ ] Infrastructure cost is documented and acceptable

---

## Later / unscheduled

Not a milestone. Track here until scheduled:

- GUI for the client
- Epic Games Store and GOG Galaxy discovery
- User guide and FAQ
- Auto-update checking
- Delta updates and compression
- Multi-language UI
- Browser extension or launcher integration

---

**Maintainer:** Simsalabim  
**Last updated:** October 24, 2026
