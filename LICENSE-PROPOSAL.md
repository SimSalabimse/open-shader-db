# License for open-shader-db

This document outlines the licensing rationale for open-shader-db.

## Decision (Approved October 24, 2026)

**Chosen approach:**
- **Client code:** MIT License (see [LICENSE](LICENSE))
- **Published data:** CC0 1.0 Universal (see [DATA-LICENSE.md](DATA-LICENSE.md))
- **If reusing any SCSKiller code:** Must adopt GPL-3.0-or-later for entire combined work

This dual-licensing approach maximizes community adoption while maintaining GPL compatibility and protecting contributors.

## Background and constraints

### SCSKiller is GPL-3.0-or-later

[SCSKiller](https://github.com/BlueHeisenberg/SCSKiller)'s codebase is licensed under GPL-3.0-or-later. This has implications:

- **If we reuse any SCSKiller code:** We MUST adopt GPL-3.0-or-later for the entire distributed combined work (not just "the derived portions"), preserve their copyright notices, include the full GPL license text, mark our modifications, and comply with any additional permissions under GPL section 7 if SCSKiller includes them. We must also ensure our project is clearly labeled as unofficial and not affiliated (trademark hygiene, not a GPL requirement).

- **If we write from-scratch code:** We may choose any license, including permissive ones like MIT or Apache-2.0. Writing a client that merely reads or generates files in a documented Microsoft format is **not** a derivative work of SCSKiller.

### Project goals

1. **Maximize adoption:** Lower barriers for games and tools to integrate
2. **Protect contributors:** Ensure data isn't locked behind future paywalls
3. **Enable commercial use:** Allow game developers to bundle or derive from this work
4. **Maintain GPL compatibility:** Keep the door open for future code sharing
5. **Protect privacy and safety:** Ensure validation code remains auditable

## Licensing options

### Option 1: MIT License (from-scratch code)

**Pros:**
- Maximum adoption – can be used in commercial games, GPL projects, proprietary tools
- Simple and well-understood by developers
- GPL-compatible: GPL projects can incorporate MIT code
- Low friction for game developers who want to integrate

**Cons:**
- Permissive: derivative works can become proprietary
- No patent grant (not a concern for this project)

**Best for:** Client code, utilities, integrations

### Option 2: Apache-2.0 (from-scratch code)

**Pros:**
- Explicit patent grant (protects contributors and users)
- Still GPL-compatible (GPLv3 and later)
- Allows commercial use
- Well-understood in industry

**Cons:**
- Slightly more complex than MIT
- Not compatible with GPLv2-only projects (irrelevant here)
- Requires NOTICE file to be preserved

**Best for:** Client code where patent clarity matters

### Option 3: GPL-3.0-or-later (if reusing SCSKiller code)

**Pros:**
- Allows reuse of SCSKiller code
- Copyleft ensures derivatives remain open
- Strong community enforcement

**Cons:**
- Game developers may be reluctant to integrate GPL code
- Cannot be used in proprietary tools or middleware
- Limits adoption by commercial projects

**Best for:** Only if we actually reuse SCSKiller's code (not just data format compatibility)

### Data licensing: CC0 vs CC-BY

The shader hash database itself is not software – it's a dataset. We need a separate data license.

#### CC0 1.0 Universal (Public Domain Dedication)

**Pros:**
- Maximum reuse: anyone can use the data for any purpose
- No attribution required (removes legal ambiguity)
- GPL-compatible
- Simple: "no rights reserved"

**Cons:**
- No attribution requirement (contributors aren't credited by default)
- Cannot be revoked

**Best for:** Maximizing data reuse and commercial adoption

#### CC-BY 4.0 (Attribution)

**Pros:**
- Requires attribution of the community database
- Still allows commercial use
- GPL-compatible (with some complexity)

**Cons:**
- Attribution requirement may complicate automated distribution
- Slightly higher friction than CC0

**Best for:** Ensuring the project receives credit

## Rationale

### **Client code: MIT License** ✅ APPROVED

**Rationale:**
- The client is a tool that reads and writes Microsoft's documented SODB format
- Maximum adoption is critical – game developers need to trust and integrate this
- MIT is simpler than Apache-2.0 for this use case, and patent concerns are minimal
- We can always add Apache-2.0 components later if patent clarity becomes important
- GPL projects can incorporate MIT code, maintaining compatibility

### **Published data: CC0 1.0 Universal** ✅ APPROVED

**Rationale:**
- The database is community-contributed factual data (description hashes and metadata)
- We want zero friction for game developers to use this data
- Attribution for a hash database is not essential to the project's goals
- CC0 removes all legal ambiguity – the data is truly free
- Projects can still credit the community voluntarily (and we encourage it)
- **Note:** CC0 does not prevent someone from putting the data behind a paywall; it only ensures we cannot stop them. Our freely available distribution competes on accessibility.

### **If we ever reuse SCSKiller code: GPL-3.0-or-later for entire combined work**

**Rationale:**
- We have no current plans to reuse SCSKiller's code
- If we ever do, we MUST comply with GPL-3.0-or-later
- We would need to:
  - Adopt GPL-3.0-or-later for the entire distributed combined work
  - Preserve SCSKiller's copyright notices
  - Include the full GPL license text
  - Check for and comply with any GPL section 7 additional permissions
  - Mark our modifications clearly
  - Maintain the "unofficial and not affiliated" disclaimer (trademark hygiene)

## Implementation

1. ✅ **LICENSE file added** – MIT License with copyright holder "Simsalabim"
2. ✅ **DATA-LICENSE.md added** – CC0 1.0 Universal for published data
3. **Add copyright headers** to new code files: `Copyright (c) 2026 Simsalabim and contributors`
4. **Add SPDX identifiers** to code files: `SPDX-License-Identifier: MIT`
5. **Document in CONTRIBUTING.md** that contributions are under MIT (code) and CC0 (data)

## Trade-offs and risks

| Decision | Trade-off | Mitigation |
|----------|-----------|------------|
| MIT over Apache-2.0 | No explicit patent grant | Patent concerns are minimal for this type of tool |
| CC0 over CC-BY | No attribution requirement | Encourage voluntary attribution in docs |
| Not using GPL | Derivatives can be proprietary | Our focus is adoption, not copyleft enforcement |
| Separate code/data licenses | Slightly more complex | Clear documentation and SPDX tags |

## Open questions for the maintainer

1. **Do you prefer MIT or Apache-2.0 for the client code?** (Recommendation: MIT for simplicity)
2. **Do you prefer CC0 or CC-BY for the database?** (Recommendation: CC0 for maximum reuse)
3. **Do you plan to reuse any SCSKiller code?** (If yes, must use GPL-3.0-or-later)
4. **Should we add a CLA or DCO for contributors?** (Recommendation: DCO for simplicity)

## Conclusion

The **MIT (code) + CC0 (data)** approach:
- Maximizes community adoption and commercial integration
- Maintains GPL compatibility for code sharing
- Protects the community data from future paywalls (irrevocable dedication)
- Keeps licensing simple and developer-friendly
- Aligns with the project's "free and open" mission

**Status:** Approved by maintainer on October 24, 2026. See [LICENSE](LICENSE) and [DATA-LICENSE.md](DATA-LICENSE.md).

---

**Author:** Simsalabim  
**Last updated:** October 24, 2026
