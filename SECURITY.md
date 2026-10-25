# Security Policy

## Reporting a vulnerability

If you discover a security vulnerability in open-shader-db, please report it responsibly. **Do not open a public GitHub issue for security vulnerabilities.**

### How to report

**GitHub Security Advisories (preferred):** Use GitHub's [private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability) feature if available for this repository.

**Email (alternative):** Send details to the maintainer at a secure email address (TBD – will be added once the project is active).

**Encrypted communication:** If available, use encryption (method TBD).

### What to include

1. **Description:** What is the vulnerability?
2. **Impact:** What can an attacker do?
3. **Reproduction:** Steps to reproduce the issue
4. **Suggested fix:** If you have ideas (optional)
5. **Disclosure timeline:** Your expectations for disclosure

### What to expect

- **Acknowledgment:** We will acknowledge your report within 72 hours
- **Updates:** We will provide status updates every 7 days
- **Fix timeline:** We aim to fix critical vulnerabilities within 30 days
- **Disclosure:** We will coordinate public disclosure with you
- **Credit:** We will credit you in the security advisory (if you wish)

### Scope

**In scope:**
- Client tool vulnerabilities (code execution, privilege escalation, data leaks)
- Ingest Worker vulnerabilities (injection, bypass, DoS)
- Validation pipeline vulnerabilities (sandbox escape, code injection)
- Distribution vulnerabilities (signature bypass, supply chain attacks)
- Privacy violations (unintended data collection or leakage)

**Out of scope:**
- Social engineering attacks
- Physical access attacks
- Vulnerabilities in third-party dependencies (report upstream, but let us know)

## Security design principles

open-shader-db is designed with security and privacy as core principles:

### Privacy-first design

- **No user accounts:** No authentication or tracking
- **No IP logging:** Ingest Worker does not store IP addresses
- **No personal data:** Only shader hashes and build identifiers collected
- **Upload preview:** Users see what will be uploaded before transmission

See [ARCHITECTURE.md](ARCHITECTURE.md#privacy-model) for details.

### Anti-poisoning measures

- **Hash whitelist:** Only accept hashes from shipped game data
- **Multi-contributor consensus:** Require ≥3 independent uploads
- **Sandboxed compilation:** Compile in isolated environment (Docker, Firecracker)
- **Signed releases:** Databases are signed and verified before use

See [ARCHITECTURE.md](ARCHITECTURE.md#trust-and-anti-poisoning-model) for details.

### Threat model

**Threats we protect against:**
1. **Malicious uploads:** Bad actors uploading invalid or malicious shader hashes
2. **Sybil attacks:** Multiple fake identities to poison consensus
3. **Supply chain attacks:** Compromised client or distribution
4. **Data poisoning:** Injecting bad data to cause crashes or exploits

**Threats we do NOT protect against:**
1. **Game developer attacks:** We assume game publishers do not intentionally ship malicious shaders
2. **OS-level compromise:** We cannot protect against rootkits or kernel exploits
3. **Physical access:** We cannot protect against attackers with physical access to contributor machines

## Supported versions

**Status:** The project is in early design. No versions are released yet.

Once released, we will support:
- **Current stable version:** Full support
- **Previous stable version:** Security fixes only for 90 days
- **Older versions:** No support (please upgrade)

## Security updates

Security fixes will be released as soon as possible:

- **Critical vulnerabilities:** Released within 7 days
- **High vulnerabilities:** Released within 30 days
- **Medium vulnerabilities:** Released in next regular release
- **Low vulnerabilities:** Released in next regular release

Security releases will be announced:
- GitHub Security Advisories
- GitHub Releases notes
- Project README (if critical)

## Secure development practices

We follow these practices to maintain security:

1. **Code review:** All changes reviewed before merging
2. **Dependency updates:** Regular updates of third-party libraries
3. **Static analysis:** Automated security scanning (TBD)
4. **Fuzzing:** Input fuzzing for parsers (TBD)
5. **Signed releases:** All official releases are signed (client binaries when available)
6. **Reproducible builds:** Community can verify official binaries (TBD)

## Known security considerations

### Client tool risks

- **Privilege escalation:** Client runs as user, not admin – limits impact
- **Code execution:** Parsing untrusted SODB files – use safe parsers, fuzzing
- **Network attacks:** HTTPS only, verify signatures, no automatic downloads

### Ingest Worker risks

- **Denial of service:** Cloudflare rate limiting (edge, temporary) + Turnstile + per-build upload cap
- **Injection attacks:** Schema validation, no SQL/command injection
- **Amplification attacks:** Limit manifest size and hash count

### Validation pipeline risks

- **Sandbox escape:** Use mature isolation (Docker, Firecracker, gVisor)
- **Resource exhaustion:** Limit compilation time and memory
- **Supply chain:** Pin dependencies, verify signatures

### Distribution risks

- **Compromised releases:** Signed releases, immutable after publishing
- **Tampered databases:** Clients verify signatures before use
- **MITM attacks:** HTTPS only, certificate pinning (TBD)

## Responsible disclosure policy

We believe in coordinated vulnerability disclosure:

1. **Private disclosure:** Report vulnerabilities privately first
2. **Fix development:** We develop a fix and test it
3. **Coordinated release:** We agree on a public disclosure date with the reporter
4. **Public disclosure:** We release the fix and publish a security advisory
5. **Credit:** We credit the reporter (if they wish)

**Typical timeline:**
- Day 0: Vulnerability reported
- Day 3: Acknowledgment sent
- Day 7-30: Fix developed and tested
- Day 30: Fix released, advisory published

## Bug bounty

**Status:** No bug bounty program at this time.

We appreciate responsible disclosure and will credit security researchers in advisories and release notes.

If the project grows, we may introduce a bug bounty program in the future.

---

## Takedown requests

A takedown request (remove a published record) is not a security vulnerability. Use the [takedown request](https://github.com/SimSalabimse/open-shader-db/issues/new?template=takedown_request.yml) issue template. Do not attach bytecode or game files. Do not use private vulnerability reporting for ordinary takedown requests.

## Questions?

If you have questions about security (non-sensitive), open a GitHub issue with the `security` label.

For sensitive security matters, use the private reporting methods above.

**Maintainer:** Simsalabim  
**Last updated:** October 24, 2026
