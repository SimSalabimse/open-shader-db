# Contributing to open-shader-db

Thank you for your interest in contributing to open-shader-db! This document provides guidelines for contributing code, documentation, data, and design feedback.

## Table of contents

- [Code of conduct](#code-of-conduct)
- [How to contribute](#how-to-contribute)
- [Contributing code](#contributing-code)
- [Contributing documentation](#contributing-documentation)
- [Contributing shader data](#contributing-shader-data)
- [Reporting bugs](#reporting-bugs)
- [Takedown requests](#takedown-requests)
- [Suggesting features](#suggesting-features)
- [Development setup](#development-setup)
- [Pull request process](#pull-request-process)
- [Licensing](#licensing)

## Code of conduct

This project is committed to fostering a welcoming and inclusive community. We expect all contributors to:

- Be respectful and considerate
- Focus on technical merit and constructive feedback
- Assume good faith in others
- Avoid harassment, discrimination, or inflammatory language

**Enforcement:** Violations should be reported to the maintainer (Simsalabim). Serious or repeated violations may result in a ban from the project.

## How to contribute

There are many ways to contribute:

1. **Report bugs:** Found a problem? Open an issue with reproduction steps
2. **Suggest features:** Have an idea? Open an issue to discuss it
3. **Improve documentation:** Fix typos, clarify explanations, add examples
4. **Write code:** Implement features, fix bugs, improve performance
5. **Share shader data:** Once the client is ready, contribute your game's shader hashes
6. **Review pull requests:** Provide constructive feedback on others' contributions
7. **Spread the word:** Tell others about the project

## Contributing code

### Before you start

1. **Check existing issues:** Someone may already be working on it
2. **Open an issue first:** Discuss large changes before implementing
3. **Read the architecture:** Understand the design in [ARCHITECTURE.md](ARCHITECTURE.md)
4. **Follow the roadmap:** See [ROADMAP.md](ROADMAP.md) for planned work

### Code guidelines

- **Language:** Client in Rust (Cargo workspace); validation language still TBD
- **Style:** Follow language-specific conventions (e.g., Rust's `rustfmt`, C++'s clang-format)
- **Comments:** Explain *why*, not *what*; avoid obvious comments
- **Tests:** Write unit tests for new functionality
- **Documentation:** Update docs when changing behavior

### Commit messages

Follow the [Conventional Commits](https://www.conventionalcommits.org/) format:

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style (formatting, no logic change)
- `refactor`: Code restructuring (no behavior change)
- `test`: Adding or updating tests
- `chore`: Build, CI, or tooling changes

**Example:**

```
feat(client): add Steam game discovery

Implement scanning of Steam library folders to auto-detect installed
games. Reads libraryfolders.vdf and resolves app manifests.

Related: Implement Steam game discovery
```

## Contributing documentation

Documentation improvements are always welcome!

- **Typos and clarity:** Fix directly and open a PR
- **Large changes:** Open an issue first to discuss scope
- **New documents:** Propose in an issue before writing
- **Markdown style:** Use [GitHub Flavored Markdown](https://github.github.com/gfm/)
- **Links:** Use relative paths for internal links (e.g., `[ARCHITECTURE](ARCHITECTURE.md)`)

## Contributing shader data

**Status:** The client tool is not yet ready. Check back later!

Once the client is available:

1. **Download the client** from GitHub Releases
2. **Verify the signature** (Minisign)
3. **Run the scan:** `open-shader-db scan`
4. **Review the upload preview:** Check what will be uploaded
5. **Submit:** Confirm the upload

**Privacy:**
- No personal data is collected
- No game paths are transmitted
- No accounts or authentication required
- See [ARCHITECTURE.md](ARCHITECTURE.md#privacy-model) for details

## Reporting bugs

Use GitHub Issues to report bugs. Include:

1. **Description:** What happened vs. what you expected
2. **Steps to reproduce:** How to trigger the bug
3. **Environment:** OS, client version, game (if applicable)
4. **Logs:** Relevant error messages or log output
5. **Screenshots:** If applicable

**Template:**

```markdown
**Bug description:**
The client crashes when scanning Epic Games Library.

**Steps to reproduce:**
1. Install open-shader-db v0.1.0
2. Run `open-shader-db scan`
3. Client crashes with error "Access violation"

**Environment:**
- OS: Windows 11 23H2
- Client: v0.1.0
- Game: None (crash during scan)

**Logs:**
[Attach client.log]
```

## Takedown requests

If you represent a rights holder and want a published record removed, open a [takedown request](https://github.com/SimSalabimse/open-shader-db/issues/new?template=takedown_request.yml). Identify the game and build and the record or hash. Do not attach shader bytecode or game files. The project will remove the record from future releases promptly and review the request in good faith.

## Suggesting features

Use GitHub Issues to suggest features. Include:

1. **Use case:** What problem does this solve?
2. **Proposed solution:** How should it work?
3. **Alternatives:** Other approaches considered
4. **Impact:** Who benefits? How critical is it?

**Template:**

```markdown
**Feature request:**
Support GOG Galaxy game discovery

**Use case:**
Many games are installed via GOG, but the client only supports Steam.

**Proposed solution:**
Parse GOG Galaxy's SQLite database to discover installed games.

**Alternatives:**
- Manual game path entry (less convenient)
- Support only Steam initially (misses GOG users)

**Impact:**
- Moderate: GOG Galaxy has meaningful user base
- Low effort: Similar to Steam discovery
```

## Development setup

**Status:** Development environment is not yet defined. This section will be updated as the project progresses.

Tentative setup (subject to change):

1. **Clone the repository:**

   ```bash
   git clone https://github.com/YOUR_USERNAME/open-shader-db.git
   cd open-shader-db
   ```

2. **Install dependencies:**

   TBD (depends on language choice)

3. **Build the project:**

   TBD

4. **Run tests:**

   TBD

5. **Run the client locally:**

   TBD

## Pull request process

1. **Fork the repository:** `https://github.com/SimSalabimse/open-shader-db`
2. **Create a feature branch:** `git checkout -b feat/my-feature`
3. **Make your changes:** Follow code guidelines
4. **Write tests:** Cover new functionality
5. **Update documentation:** Reflect changes in docs
6. **Commit:** Use conventional commit format
7. **Push:** `git push origin feat/my-feature`
8. **Open a pull request:** Provide a clear description
9. **Respond to feedback:** Address review comments
10. **Merge:** Maintainer will merge when approved

**Pull request template:**

```markdown
## Description
Briefly describe the changes in this PR.

## Related issue
Related: Implement Epic Games Store and GOG Galaxy game discovery

## Type of change
- [ ] Bug fix
- [ ] New feature
- [ ] Documentation update
- [ ] Refactoring

## Checklist
- [ ] Tests pass locally
- [ ] Added tests for new functionality
- [ ] Updated relevant documentation
- [ ] Followed code style guidelines
- [ ] Commit messages follow conventional commits
```

## Licensing

By contributing to open-shader-db, you agree that:

- **Code contributions** are licensed under the project's MIT code license.
- **Published data contributions** (shader hashes, metadata, verification records, and similar records) are licensed under CC0 1.0 Universal.
- If you upload data, you confirm that you have the right to do so and that you do not reserve any rights that conflict with the CC0 dedication for the published data.

**Source file header:** add this exact SPDX line at the top of every new source file:

```text
SPDX-License-Identifier: MIT
```

**Developer Certificate of Origin (DCO):**

We may adopt the [DCO](https://developercertificate.org/) in the future. For now, by submitting a pull request, you certify that:

- You created the contribution, or
- You have the right to submit it under the project's license, and
- You understand it will be distributed under the project's license

---

## Questions?

If you have questions about contributing, please:

1. Check the [README](README.md) and [ARCHITECTURE](ARCHITECTURE.md)
2. Search existing [GitHub Issues](https://github.com/SimSalabimse/open-shader-db/issues)
3. Open a new issue with the `question` label

Thank you for contributing to open-shader-db!

**Maintainer:** Simsalabim  
**Last updated:** October 24, 2026
