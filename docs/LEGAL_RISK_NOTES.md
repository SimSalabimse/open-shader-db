# open-shader-db: legal-risk research notes (issue #8)

> **Research notes, not legal advice. Written by an automated assistant from public sources on Sunday 25 October 2026 (CET, Europe/Oslo). No lawyer reviewed this.**

On 2026-10-25, the maintainer (Simsalabim) decided to proceed with the whole roadmap without asking a legal help group, accepting the residual risks listed below. The project keeps these design rules: no bytecode ever; hash keys; opt-in only; no accounts; no stored IPs; no game paths; upload preview; offline extraction only; never hook or attach to game processes (anti-cheat); never bundle the preview Agility SDK or any Microsoft redistributable; no Microsoft or storefront trademarks as branding.

Scope: the five questions in `ARCHITECTURE.md` → "Legal and ToS doubts". Maintainer: Simsalabim. Every quote below comes from a page I opened on 25 Oct 2026, and the links are in the Sources list. **UNVERIFIED** marks anything I could not check against a primary source.

---

## 1. Microsoft licences and terms

- **The specs are openly licensed.** The DirectX-Specs repo licenses its documentation under CC-BY-4.0 and its code under MIT [S1, S2]. That covers the State Object Database (SODB) spec page, which publishes the full SQL schema [S3]. Two limits apply. The licences "do not grant you rights to use any Microsoft names, logos, or trademarks", and Microsoft and contributors "reserve all other rights, whether under their respective copyrights, patents, or trademarks" [S2]. So we can reuse the schema text with attribution, but no patent licence is stated. Patents are **UNVERIFIED** (I did no patent search).
- **The samples are MIT** ("Copyright (c) 2015 Microsoft") [S4].
- **The spec defines the SODB as including DXIL.** It describes the SODB as "The serialized SO/PSO description objects and the shader DXIL needed to reconstruct any create SO/PSO API calls" [S3]. Because we strip DXIL, our records are not full SODBs.
- **Microsoft assumes the storefront delivers SODBs.** The ASD docs say "Using ASD requires a supported Windows version, an ASD-capable GPU and driver, and storefront integration", and the last stage is "Deploy the SODB to a storefront" [S5]. The sample readme says "You submit your SODB alongside your game package" and "The store runs a server-side compile…" [S6]. **None of the Microsoft pages I read mentions third-party or community SODBs, either to allow them or to forbid them.**
- **The Agility SDK preview licence is restrictive.** It is the NuGet package `Microsoft.Direct3D.D3D12` 1.721.3-preview [S7]. Under §4 you may not "work around any technical limitations in the software", may not "reverse engineer, decompile or disassemble the software", and may not "share, publish, distribute, or lease the software". a "TIME-SENSITIVE SOFTWARE" clause makes it time-limited. **Do not bundle or redistribute the preview SDK DLLs or tools**, and have users install them from Microsoft.

## 2. Store and publisher EULAs

**Both citations in `ARCHITECTURE.md` are wrong and need fixing:**

| ARCHITECTURE.md says | What the current document says |
|---|---|
| Steam SSA "§ 2.C … reverse engineer" | §2.C is "License to Use Valve Developer Tools". The reverse-engineering ban is in **§2.G** "Restrictions on Use of Content and Services" (SSA last updated 10 Sep 2026) [S8] |
| Epic EULA "§ 3 … reverse engineer" | §3 is "Updates and Patches". The ban is in **§2 "License Conditions", item (e)** (EULA last updated 15 Jan 2025) [S9] |

Short exact phrases:

- **Steam §2.G** [S8]: "Except as otherwise permitted under this Agreement …, or under applicable law notwithstanding these restrictions, you may not … reverse engineer, derive source code from, modify, disassemble, decompile, create derivative works based on … the Content and Services or any software accessed via Steam without the prior consent, in writing, of Valve." §2.A limits use to "personal, non-commercial use". §4.B covers cheats: you must not "disable, circumvent, or otherwise interfere with the operation of software designed to prevent or report the use of Cheats."
- **Epic §2(e)** [S9]: you may not "reverse engineer, derive source code from, modify, adapt, translate, decompile, or disassemble it or make derivative works based on it". "Software" includes "video games or other software made available for download or use through the Epic Games Store". §18: "This Agreement does not change your rights under the laws of your jurisdiction if the laws of your jurisdiction do not permit it to do so."
- **GOG User Agreement §11.1** (effective 9 Mar 2026) [S10]:
  - (b) rights in game content depend on "what the GOG content rights holder allows you to do (GOG can't grant such rights)".
  - (c) no reverse engineering of GOG services "unless you're allowed in this Agreement or by the law in your country".
  - (e) bans "extraction tools or other software that interact with or affect GOG services or GOG content in any way". **Clause (e) is the broadest one I found, and it names extraction tools directly.**
- **EA User Agreement §2** (14 May 2026) [S11]: "You may not reverse engineer or attempt to extract or otherwise use source code or other data from EA Services, unless expressly authorized by EA or permitted by law."
- **Ubisoft EULA §1.2** (01/2023) [S12]: no right to "reverse engineer, decompile, disassemble, adapt, reproduce, or create derivate works of this Product". Section 1 also describes monitoring for "unauthorized third party programs".
- **Activision Terms of Use §3.A** (5 Sep 2025) [S13]: bans "decompile, reverse engineer, disassemble, … modify, or create derivative works of the Product … (except as and only to the extent allowed by applicable law …)".
- **Take-Two Terms of Service §2.3 and §6.4(4)** (28 Feb 2025) [S14]: "reverse engineer (except where permitted under applicable legal exceptions deriving from EU Directive 2009/24 or other applicable law)". This is the only one with an explicit EU interoperability carve-out.

What they share: every one bans reverse engineering, and most of them (Steam, Epic, GOG, EA, Activision, Take-Two) say "except where law permits". Our case depends on that carve-out, so it matters a great deal.

## 3. Law and precedent

- **US: ideas and methods.** Under 17 U.S.C. §102(b), copyright does not extend to any "idea, procedure, process, system, method of operation" [S15]. Pipeline state records (blend factors, cull mode, formats) look like parameters for a method of operation. **No court has ruled on PSO descriptors specifically, so this is UNVERIFIED.**
- **Google v. Oracle (2021)** [S16]. The Court did *not* decide copyrightability: "we assume, for argument's sake, that the material was copyrightable". It held that copying an API "to reimplement a user interface, taking only what was needed … constituted a fair use of that material as a matter of law". The case helps, but fair use turns on the facts, and it says nothing about contract (EULA) claims.
- **Sega v. Accolade, and Sony v. Connectix** (9th Cir.), as summarised by the EFF [S17]. Disassembly to find functional interface information was fair use in both. The EFF also notes, for Sega, "Sega sold the Genesis without any license agreements". For Connectix it says "the case did not involve any license agreements". **I could not open the primary opinions (Justia returned 403), so their exact wording is UNVERIFIED.**
- **The counterweight: Davidson & Associates (Blizzard) v. Jung (BnetD), 8th Cir.**, per the EFF [S17]. Click-through EULA bans on reverse engineering were enforced, and "the programmers waived their fair use rights through the EULA". **In the US, a EULA can override fair use.**
- **DMCA §1201(f)** [S18]. It permits circumvention "for the sole purpose of identifying and analyzing those elements of the program that are necessary to achieve interoperability of an independently created computer program". Information may be shared "solely for the purpose of enabling interoperability" and only where that "does not constitute infringement … or violate applicable law other than this section". §1201 only matters if we bypass a technical protection measure (encryption or DRM). If we read unencrypted files the user already has, it is probably not engaged. That is **UNVERIFIED** for any specific game that encrypts its shader caches or packages.
- **EU Software Directive 2009/24/EC** [S19–S21]. I read the copy on legislation.gov.uk because EUR-Lex blocked automated access.
  - Art 5(3): a lawful user may "observe, study or test the functioning of the program in order to determine the ideas and principles".
  - Art 6(1): decompilation is allowed where "indispensable to obtain the information necessary to achieve the interoperability of an independently created computer program", for a lawful user, where the information is not readily available, and only for the necessary parts.
  - Art 6(2): the information must not be "given to others, except when necessary for the interoperability".
  - **Art 8: "Any contractual provisions contrary to Article 6 or to the exceptions provided for in Article 5(2) and (3) shall be null and void."** In the EU/EEA, unlike the US, the EULA cannot remove these rights.
  - The catch: Art 6(2) limits *sharing*, and a public database is sharing. Does a public database count as "necessary for the interoperability"? That is the central open question, and the answer is **unknown**.
- **Norway (EEA).** Åndsverkloven §42 "Omvendt utvikling av datamaskinprogram" implements Art 6 and ends "Bestemmelsene i denne paragrafen kan ikke fravikes ved avtale" (the provisions cannot be overridden by contract). §24 is the database right [S22].
- **EU Database Directive 96/9/EC** [S23–S25].
  - Art 1(2) defines a database as "a collection of independent works, data or other materials arranged in a systematic or methodical way and individually accessible". A game's SODB fits that definition.
  - Art 7(1) gives a sui generis right to a maker with "substantial investment in either the obtaining, verification or presentation of the contents", against "extraction and/or re-utilization of the whole or of a substantial part".
  - Our hub would hold a *substantial part* of each game's state table. The risk depends on whether the publisher's investment counts as "obtaining … the contents" or merely creating them (the BHB v William Hill line). **I could not open the CJEU judgments (curia and EUR-Lex were blocked), so that point is UNVERIFIED.** The same goes for SAS Institute v WPL (C-406/10) on data file formats.

## 4. Design choices that lower risk, and the risks that remain

**What lowers risk** (from the M1 design):
- No DXIL is shared, so no executable code is distributed.
- Records are keyed by the SHA-256 of the DXIL plus the game build, so a record is useless without a lawfully owned copy.
- The SODB is rebuilt locally from the user's own files.
- Contributors opt in, and there are no accounts, IPs or game paths.
- Code is MIT and data is CC0.
- No game files are modified, and no SDK binaries are bundled.

**Risks that remain:**
1. **US contract risk.** EULA bans plus BnetD mean users and contributors may breach terms even if copyright law allows the activity. The main exposure is the account bans the stores reserve, not damages. That last point is my assessment, not a sourced fact.
2. **GOG §11.1(e)** names "extraction tools" directly.
3. **Database right.** Aggregating a substantial part of a game's state table and re-using it in public.
4. **Art 6(2) sharing limit.** Public distribution may exceed "necessary for the interoperability".
5. **Anti-cheat.** Any tool that hooks D3D12 inside a protected game process could trip anti-cheat (Steam §4.B, Ubisoft's RAM monitoring). Only offline extraction with the game closed avoids this. Whether a given anti-cheat flags local PSDB registration is **UNVERIFIED**.
6. **Storefront expectation.** Microsoft's docs assume storefront delivery [S5, S6]. Nothing I found forbids local use, but nothing endorses it either.
7. **Trademarks.** Do not use "DirectX" or the Microsoft or store names as project branding [S2].

## 5. Should Simsalabim ask a free open-source legal help group?

**Proposal: yes, before any public hosting or sharing (M2+). Local-only M1 does not need to wait.** All of the questions turn on EU/EEA versus US contract law, which needs a real lawyer. Candidates whose pages I opened:

| Group | What it does (from its page) | Eligibility / caveats |
|---|---|---|
| **EFF**: Legal Assistance [S26], Coders' Rights FAQ [S17] | Takes a small number of high-impact digital-rights cases and otherwise may refer people to its Cooperating Attorneys list. It "can try to refer you for pro bono (free) representation". | Mainly US courts. It avoids cases outside its specialties. "only able to take on the cases of a small percentage". **The best fit for the reverse-engineering questions.** |
| **SFLC** [S27, S28] | Services for FOSS projects: licensing, trademarks, non-profit setup, patent defence, licence defence. For legal advice, email help@softwarefreedom.org. | US-based. Its pages do not say whether help is free (**UNVERIFIED**), and its newest home-page news is from 2022. |
| **Software Freedom Conservancy** [S29, S30] | A 501(c)(3) charity that provides a non-profit home and services to member projects. | You apply as a member project. It is "selective" and "often refer[s] projects to other fiscal sponsors". It is not a legal advice desk. |
| **FSFE**, Legal / Licence Questions / Legal Network [S31] | Volunteer licence-question mailing list, plus a network of "several hundreds" of FOSS legal experts. | European. It focuses on licences and compliance, but **could point to an EEA lawyer for Art 6 and database-right questions.** |
| **OSI** [S32] | Licence approval and the Open Source Definition. | "the OSI is not a legal services organization and does not provide legal advice". **Not suitable.** |
| **Linux Foundation** [S33] | A "Neutral home" that hosts projects and runs support programmes. | `linuxfoundation.org/legal` returned 404, and I found no public legal-help service for outside projects. **Not suitable.** |
| **Univ. of Colorado, Samuelson-Glushko TLPC** [S34] | Students do public-interest tech-policy advocacy before agencies. | Its page describes policy advocacy, not client intake for individual projects. **Fit UNVERIFIED.** |
| **UC Berkeley, Samuelson Clinic** [S35] | "Our students represent public interest clients" on civil liberties, IP and access to information. | No eligibility criteria stated on the page. **Fit UNVERIFIED.** |

I also tried Jussbuss (Norwegian student legal aid), but the site did not load, so it is not listed.

**Suggested order: FSFE Legal Network (EEA law) and EFF (US) first.** Any contact must come from Simsalabim, because I contacted no one.

## 6. Summary table: PROPOSAL ONLY, Simsalabim decides

| # | Question (ARCHITECTURE.md) | Risk | Main reason | Proposed decision |
|---|---|---|---|---|
| 1 | Are pipeline state descriptors copyrightable? | **Low** (individual records) / **Medium** (the full per-game collection) | §102(b) and Art 5(3) treat ideas and methods as unprotected, and Oracle supports functional reuse. The EU database right may cover a substantial part of a game's table. | Accept as residual risk for single records. **Needs lawyer** for bulk per-game publication. |
| 2 | Does extracting from installed files violate EULAs? | **Medium** (US) / **Low–Medium** (EU/EEA) | All stores ban reverse engineering. BnetD enforces this in the US. In the EU, Art 8 (Norway §42) voids bans on Art 6 decompilation, but GOG §11.1(e) bans "extraction tools". | **Needs lawyer.** Fix the ARCHITECTURE.md citations now. |
| 3 | Does local extraction conflict with ASD storefront expectations? | **Unknown** | Microsoft documents only storefront delivery and is silent on local or community use. The preview SDK licence bars redistribution. | Accept as residual risk for local M1 (no SDK bundling). **Ask Microsoft before M2.** |
| 4 | Does publishing state descriptors count as "distributing game assets"? | **Medium** | No DXIL and hash keying help. Art 6(2) limits sharing to what is "necessary", and the database right covers re-utilisation. | **Needs lawyer** before any public hub. |
| 5 | Does this comply with anti-cheat systems? | **Medium–High** if hooking game processes / **Low** if fully offline | Steam §4.B, Ubisoft memory monitoring. Per-title behaviour is unverified. | **Avoid** in-process hooking of anti-cheat titles. Accept offline-only. |

---

## Sources (all opened Sunday 25 Oct 2026; I checked every link with `curl -sIL`)

- S1 https://github.com/microsoft/DirectX-Specs/blob/master/LICENSE
- S2 https://github.com/microsoft/DirectX-Specs
- S3 https://microsoft.github.io/DirectX-Specs/d3d/StateObjectDatabase.html
- S4 https://github.com/microsoft/DirectX-Graphics-Samples/blob/master/LICENSE
- S5 https://learn.microsoft.com/en-us/windows/win32/direct3d12/advanced-shader-delivery
- S6 https://github.com/microsoft/DirectX-Graphics-Samples/tree/master/Samples/Desktop/D3D12AdvancedShaderDelivery
- S7 https://www.nuget.org/packages/Microsoft.Direct3D.D3D12/1.721.3-preview/license (a HEAD request gets 404 but a normal GET gets 200)
- S8 https://store.steampowered.com/subscriber_agreement/?l=english
- S9 https://legal.epicgames.com/store/eula
- S10 https://support.gog.com/api/v2/help_center/en-us/articles/212632089.json (GET 200; a HEAD request gets 404). This is the API copy of the public page https://support.gog.com/hc/en-us/articles/212632089-GOG-User-Agreement , which returns 403 to automated clients (Cloudflare) but is the human-readable link
- S11 https://tos.ea.com/legalapp/WEBTERMS/US/en/PC/
- S12 https://www.ubisoft.com/legal/documents/eula/en-US
- S13 https://www.activision.com/legal/terms-of-use
- S14 https://www.take2games.com/legal/en-US/
- S15 https://www.law.cornell.edu/uscode/text/17/102
- S16 https://www.supremecourt.gov/opinions/20pdf/18-956_d18f.pdf
- S17 https://www.eff.org/issues/coders/reverse-engineering-faq
- S18 https://www.law.cornell.edu/uscode/text/17/1201
- S19 https://www.legislation.gov.uk/eudr/2009/24/article/5
- S20 https://www.legislation.gov.uk/eudr/2009/24/article/6
- S21 https://www.legislation.gov.uk/eudr/2009/24/article/8
- S22 https://lovdata.no/dokument/NL/lov/2018-06-15-40
- S23 https://www.legislation.gov.uk/eudr/1996/9/article/1
- S24 https://www.legislation.gov.uk/eudr/1996/9/article/3
- S25 https://www.legislation.gov.uk/eudr/1996/9/article/7
- S26 https://www.eff.org/pages/legal-assistance
- S27 https://softwarefreedom.org/services/
- S28 https://softwarefreedom.org/about/contact/
- S29 https://sfconservancy.org/about/
- S30 https://sfconservancy.org/projects/apply/
- S31 https://fsfe.org/activities/legal.html
- S32 https://opensource.org/faq
- S33 https://www.linuxfoundation.org/about
- S34 https://www.colorado.edu/law/academics/clinics/samuelson-glushko-technology-law-policy-clinic
- S35 https://www.law.berkeley.edu/experiential/clinics/samuelson-law-technology-public-policy-clinic/

**Tried but not opened, so not cited as sources:**
- EUR-Lex (JS challenge)
- curia.europa.eu / infocuria and BAILII (blocked), for the BHB v William Hill and SAS v WPL judgments
- Justia (403), for the Sega and Connectix opinions
- jussbuss.no (no response)

I left out "how SCSKiller-style tools exist in practice" because I did not research it from public pages.
