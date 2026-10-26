# GitHub Issues Draft

Proposed issues for the six GitHub milestones. Create them after the documentation pull request is merged. Do not invent issue numbers here; depend on other issues **by title**.

**Created:** October 24, 2026  
**Status:** Pending creation

**Allowed labels only:** `docs`, `client`, `ingest`, `validation`, `security`, `release`, `good first issue`, `decision`

**Milestone names (exact):**

- M0 Index and coverage reports
- M1 Pipeline-state records
- M2 Upload preview and submission
- M3 Consensus and publishing
- M4 Signed releases
- M5 Windows validation

---

## M0 Index and coverage reports

### Issue: Record SPDX identifiers and contribution licence statements

**Labels:** `docs`, `good first issue`

**Milestone:** M0 Index and coverage reports

**Outcome:** Code and data contribution statements match the approved MIT + CC0 licences.

**Depends on:** none (LICENSE and DATA-LICENSE.md already exist in the documentation PR)

**Acceptance criteria:**

- [ ] New source files will use `SPDX-License-Identifier: MIT` when code lands
- [ ] CONTRIBUTING.md states MIT for code and CC0 for published data
- [ ] README links LICENSE and DATA-LICENSE.md

---

### Issue: Set up CI/CD pipeline with GitHub Actions

**Labels:** `docs`, `good first issue`

**Milestone:** M0 Index and coverage reports

**Outcome:** Pull requests run Markdown and YAML checks automatically.

**Depends on:** none

**Acceptance criteria:**

- [ ] A workflow runs on pull requests
- [ ] Markdown lint and YAML validation run
- [ ] README can show a CI status badge

---

### Issue: Create GitHub issue templates and PR template

**Labels:** `docs`, `good first issue`

**Milestone:** M0 Index and coverage reports

**Outcome:** New issues and pull requests start from templates that match CONTRIBUTING.md.

**Depends on:** none

**Acceptance criteria:**

- [ ] Bug, feature, and question templates exist under `.github/ISSUE_TEMPLATE/`
- [ ] A pull request template exists
- [ ] Templates appear in the GitHub UI

---

### Issue: Choose client language and framework

**Labels:** `decision`, `client`

**Milestone:** M0 Index and coverage reports

**Outcome:** The client language and build system are chosen and documented.

**Depends on:** none

**Acceptance criteria:**

- [ ] Choice among C++, Rust, or C# is written down with rationale
- [ ] Build system is named (CMake, Cargo, or MSBuild)
- [ ] A minimal program compiles on Windows

---

### Issue: Implement SODB file parser

**Labels:** `client`

**Milestone:** M0 Index and coverage reports

**Outcome:** The client opens an SODB SQLite file and extracts description hashes without reading DXIL as publishable content.

**Depends on:** Choose client language and framework

**Acceptance criteria:**

- [ ] Parser opens files with `user_version=2` and `application_id=0xD3D50DB`
- [ ] Parser reads `app_id`, `pipeline_states`, `state_objects`, and `shader_bytecode` keys
- [ ] Parser emits description hashes and bytecode hashes (hashes only)
- [ ] Malformed files fail without crashing
- [ ] Unit tests use fixture SODB files

---

### Issue: Implement Steam game discovery

**Labels:** `client`

**Milestone:** M0 Index and coverage reports

**Outcome:** The client finds Steam-installed games and reports `.sodb` files when a scan root actually contains them, without putting install paths in uploads. A Steam install is not expected to contain those files today (store delivery was announced as "soon", with no date). See [SODB_SOURCES.md](SODB_SOURCES.md).

**Depends on:** Choose client language and framework

**Acceptance criteria:**

- [ ] Client locates Steam library folders
- [ ] Client lists installed games it can see
- [ ] Client finds SODB files when a scanned folder actually contains them
- [ ] A Steam library with no `.sodb` files is a normal result, reported once, not an error per game
- [ ] Missing Steam install is handled without a crash
- [ ] Game paths never appear in an upload payload

---

### Issue: Implement coverage comparison CLI

**Labels:** `client`

**Milestone:** M0 Index and coverage reports

**Outcome:** A user can compare a local SODB to a community coverage index and see a coverage report.

**Depends on:** Implement SODB file parser; Implement Steam game discovery

**Acceptance criteria:**

- [ ] CLI loads a local SODB and a coverage index file
- [ ] Report shows match rate and unmatched description hashes
- [ ] No personal data is written into the report
- [ ] Signature verification is not required until M4 Signed releases (unsigned local files are allowed for this issue)

---

## M1 Pipeline-state records

Legal review is required before implementation. See [ARCHITECTURE.md](../ARCHITECTURE.md#legal-and-tos-doubts).

### Issue: Complete legal review for pipeline-state records

**Labels:** `decision`, `security`

**Milestone:** M1 Pipeline-state records

**Outcome:** A written go / no-go on publishing state descriptors and extracting shaders from installed games.

**Depends on:** none

**Acceptance criteria:**

- [ ] Each item in ARCHITECTURE.md "Legal and ToS doubts" is answered or explicitly accepted as residual risk
- [ ] The decision is recorded in ARCHITECTURE.md or a follow-up note
- [ ] No M1 implementation issue is started without this outcome

---

### Issue: Implement shader extraction and matching

**Labels:** `client`, `security`

**Milestone:** M1 Pipeline-state records

**Outcome:** The client hashes DXIL from the user's installed game files and matches those hashes to published records.

**Depends on:** Complete legal review for pipeline-state records; Implement SODB file parser

**Acceptance criteria:**

- [ ] Client extracts DXIL from at least one supported game without modifying game files
- [ ] Each blob is hashed with SHA256
- [ ] Hashes match published PipelineStateRecord keys when the same shader is present
- [ ] Missing or corrupt shaders produce a clear error
- [ ] No bytecode is uploaded

---

### Issue: Implement SODB rebuild from records and local shaders

**Labels:** `client`

**Milestone:** M1 Pipeline-state records

**Outcome:** The client writes a valid SODB by combining published state records with locally extracted shaders.

**Depends on:** Implement shader extraction and matching

**Acceptance criteria:**

- [ ] Client loads PipelineStateRecord JSON
- [ ] Client joins records to local shaders by bytecode hash
- [ ] Output is a SQLite SODB with the documented schema
- [ ] Rebuild is documented; unverified matching details stay marked unverified

---

## M2 Upload preview and submission

### Issue: Implement upload manifest generation and preview

**Labels:** `client`

**Milestone:** M2 Upload preview and submission

**Outcome:** The client builds an anonymized UploadManifest and shows it before any network send.

**Depends on:** Implement SODB file parser; Implement Steam game discovery

**Acceptance criteria:**

- [ ] Manifest includes BuildId, description hashes, bytecode hashes, client version, rounded-to-hour timestamp
- [ ] Manifest includes a proof-of-ownership field
- [ ] Preview lists exactly the fields that will be sent
- [ ] No game path, username, IP, or full timestamp is included
- [ ] User can export the JSON for inspection

---

### Issue: Implement Cloudflare Worker for upload ingestion

**Labels:** `ingest`

**Milestone:** M2 Upload preview and submission

**Outcome:** A Worker accepts valid manifests over HTTPS and writes them to staging.

**Depends on:** Implement upload manifest generation and preview

**Acceptance criteria:**

- [ ] POST endpoint validates schema
- [ ] Invalid payloads return a clear error
- [ ] Cloudflare rate limiting and Turnstile are configured (edge only; not stored)
- [ ] Per-build upload cap is enforced
- [ ] Logpush omits ClientIP
- [ ] Worker does not persist IP addresses or fingerprints

---

### Issue: Set up staging storage

**Labels:** `ingest`

**Milestone:** M2 Upload preview and submission

**Outcome:** Accepted manifests land in Cloudflare R2 (or as reviewed pull requests) and expire after 90 days.

**Depends on:** Implement Cloudflare Worker for upload ingestion

**Acceptance criteria:**

- [ ] Staging path is organized by game and BuildId
- [ ] Lifecycle expires objects after 90 days
- [ ] Access policy is documented
- [ ] Alternative reviewed-PR path is documented if R2 is not used

---

## M3 Consensus and publishing

### Issue: Implement staging data loader

**Labels:** `validation`

**Milestone:** M3 Consensus and publishing

**Outcome:** All staging manifests for one BuildId can be loaded and grouped.

**Depends on:** Set up staging storage

**Acceptance criteria:**

- [ ] Loader reads every manifest for a given exe hash / BuildId
- [ ] Malformed files are skipped and logged
- [ ] Unit tests cover empty and mixed-valid staging sets

---

### Issue: Implement consensus builder

**Labels:** `validation`

**Milestone:** M3 Consensus and publishing

**Outcome:** A configurable upload threshold produces a CoverageIndex (and later record set) from agreeing content hashes.

**Depends on:** Implement staging data loader

**Acceptance criteria:**

- [ ] Agreement is on the hash of full description plus bytecode hash, not on app keys
- [ ] Threshold is configurable and documented; it is not lowered for small games
- [ ] Independence of uploaders is not claimed (no accounts)
- [ ] Rejected hashes are logged with counts

---

### Issue: Implement proof-of-ownership checks, anomaly detection, and review queue

**Labels:** `validation`, `security`

**Milestone:** M3 Consensus and publishing

**Outcome:** Uploads that fail ownership checks or look anomalous are held for human review; reviewed PRs remain the trust fallback.

**Depends on:** Implement consensus builder

**Acceptance criteria:**

- [ ] Server verifies the proof-of-ownership field
- [ ] Suspicious timing or identical bursts are flagged
- [ ] Reviewers can approve or reject
- [ ] Reviewed GitHub PRs are accepted as a fallback ingest path

---

### Issue: Implement index and record generation for publishing

**Labels:** `validation`

**Milestone:** M3 Consensus and publishing

**Outcome:** Consensus data is written as CoverageIndex JSON (M0) and, after M1, PipelineStateRecord JSON, ready for signing.

**Depends on:** Implement consensus builder

**Acceptance criteria:**

- [ ] Output schemas match ARCHITECTURE.md
- [ ] Files go to a staging publish area, not directly to a signed release
- [ ] Metadata includes upload count, consensus count, and generation time

---

## M4 Signed releases

### Issue: Implement Minisign signing for releases

**Labels:** `release`, `security`

**Milestone:** M4 Signed releases

**Outcome:** Published files are signed with a dedicated offline Minisign project key.

**Depends on:** Implement index and record generation for publishing

**Acceptance criteria:**

- [ ] Detached `.minisig` files are produced
- [ ] SHA256 sidecar files are produced
- [ ] Public key is in the repository
- [ ] Private key is offline; rotation is documented
- [ ] Sigstore remains a short "considered" note only, not the implementation

---

### Issue: Automate GitHub Releases upload

**Labels:** `release`

**Milestone:** M4 Signed releases

**Outcome:** A tagged release attaches data, signature, and checksum and updates the download catalog.

**Depends on:** Implement Minisign signing for releases

**Acceptance criteria:**

- [ ] Workflow uploads the data file, `.minisig`, and `.sha256`
- [ ] Catalog JSON lists the new release
- [ ] Release notes are generated

---

### Issue: Implement client Minisign verification and signed download

**Labels:** `client`, `security`

**Milestone:** M4 Signed releases

**Outcome:** The client downloads catalog entries, verifies Minisign, and only then uses the file for coverage (or later rebuild).

**Depends on:** Implement Minisign signing for releases; Implement coverage comparison CLI

**Acceptance criteria:**

- [ ] Invalid signatures are rejected
- [ ] SHA256 mismatch is rejected
- [ ] After verify, M0 coverage comparison runs against the signed index

---

## M5 Windows validation

### Issue: Prototype Windows GPU compile validation

**Labels:** `validation`, `security`

**Milestone:** M5 Windows validation

**Outcome:** A documented, optional Windows + GPU path can compile a rebuilt SODB toward a PSDB. This milestone is deferred; M0 through M4 must not wait on it.

**Depends on:** Implement SODB rebuild from records and local shaders

**Acceptance criteria:**

- [ ] Approach is a Windows sandbox, VM, or AppContainer (not a Linux GitHub Actions GPU compile)
- [ ] Success and failure are recorded
- [ ] Cost and host requirements are written down
- [ ] This path is not required to publish an M4 signed release

---

## Later / unscheduled (not a milestone)

Do not assign these to M0–M5.

### Issue: Implement Epic Games Store and GOG Galaxy game discovery

**Labels:** `client`

**Milestone:** none (later / unscheduled)

**Outcome:** Discovery works for Epic and GOG the same way Steam discovery works for M0.

**Depends on:** Implement Steam game discovery

**Acceptance criteria:**

- [ ] Epic and GOG installs are listed when present
- [ ] No paths appear in upload manifests

---

### Issue: Build GUI for client tool

**Labels:** `client`

**Milestone:** none (later / unscheduled)

**Outcome:** Non-technical users can run scan, preview, and coverage from a window.

**Depends on:** Implement coverage comparison CLI; Implement upload manifest generation and preview

**Acceptance criteria:**

- [ ] GUI covers the CLI features that exist at the time
- [ ] Upload preview is visible before send
- [ ] Settings persist between runs

---

### Issue: Write user guide and FAQ

**Labels:** `docs`, `good first issue`

**Milestone:** none (later / unscheduled)

**Outcome:** End-user docs cover install, coverage, upload preview, and how to verify Minisign.

**Depends on:** Implement coverage comparison CLI

**Acceptance criteria:**

- [ ] Guide and FAQ live under `docs/`
- [ ] Privacy promises match ARCHITECTURE.md
- [ ] Signature verification steps use Minisign

---

## Summary

**Milestone issues:** 20

| Milestone | Count |
|-----------|-------|
| M0 Index and coverage reports | 7 |
| M1 Pipeline-state records | 3 |
| M2 Upload preview and submission | 3 |
| M3 Consensus and publishing | 4 |
| M4 Signed releases | 3 |
| M5 Windows validation | 1 |

**Later / unscheduled:** 3 (not milestone issues)

**By label (milestone issues only, an issue may have two labels):**

- `client`: 8
- `docs`: 3
- `validation`: 5
- `ingest`: 2
- `release`: 2
- `security`: 6
- `decision`: 2
- `good first issue`: 3

---

**Instructions:** After the documentation pull request merges, create GitHub issues from this file, assign the exact milestone name, and apply only the allowed labels.

**Maintainer:** Simsalabim  
**Last updated:** October 24, 2026
