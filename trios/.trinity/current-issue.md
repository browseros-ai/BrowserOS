---
issue: "#T27-EPIC-001"
title: "trios T27 automation epic: seal all BR-OUTPUT Swift behind spec-driven agents"
branch: "feat/t27-automation"
status: active
---

# trios T27 Automation Epic

**Goal:** migrate all hand-written BR-OUTPUT Swift code in trios to a T27-governed, spec-first model where agent instructions are the source of truth and direct hand edits to canon files require Agent V waiver.

## Scope

- [x] Port T27 constitution, agents, skills, coordination law (commit 056bbaf5).
- [ ] Pilot: make `RecursionGuard.swift` fully spec-driven and re-seal.
- [ ] Expand to all BR-OUTPUT Swift files.
- [ ] Govern Rust rings with T27 specs.
- [ ] Enforce via hooks, queue, claims, and verdicts.

## First Pilot

Make `BR-OUTPUT/RecursionGuard.swift` the first canon file with:
- a spec in `.trinity/specs/recursion-guard.md`,
- implementation by t27-creator,
- verification by t27-verifier,
- seal via `/t27-tri-pipeline seal`,
- experience saved via `/t27-experience-save`.

## Notes

GitHub issue number is a placeholder until `gh auth login` succeeds; replace `#T27-EPIC-001` with the real issue number before final PR.
