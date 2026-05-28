# SOUL — Trinity Constitutional Law (trios)

**Canonical location:** this file at `.trinity/SOUL.md` is the single source of truth for trios constitutional law.

**Immutable Document — Amendments Require Unanimous Architectural Consent**

> *"A specification without tests is a lie told in the future tense."*
> — Trinity TDD Axiom #1

---

## Preamble

Trios is a **Swift-first** macOS application serving as the human interface to the Trinity A2A agent network. This document establishes the constitutional principles that govern all trios development, agent behavior, and experience preservation.

---

## Article I: The Language Policy

### §1.1 ASCII-Only Source Files
**Source files MUST be ASCII-only.** Identifiers and comments MUST be English.

All files in the following categories MUST contain only ASCII characters (U+0000–U+007F):
- `.swift` — Swift source code
- Build scripts, makefiles, etc.

**FORBIDDEN in source files:**
- **Cyrillic** (U+0400–U+04FF) and other non-Latin scripts in identifiers and comments
- **Non-Latin scripts**: Greek, Arabic, Chinese, Japanese, Korean, etc., unless an Architect-approved exception exists

### §1.2 First-party documentation language
Markdown under `docs/`, `.claude/`, `.trinity/`, and root project Markdown (`README.md`, `AGENTS.md`, `CLAUDE.md`) **MUST be English**.

### §1.3 Enforcement
CI runs language checks on pull requests. The parser/build rejects non-ASCII identifiers.

### §1.4 Rationale
1. **Universality**: ASCII is universally supported across all platforms and tools
2. **Clarity**: English is the single review language for Trinity first-party docs and specs
3. **Git Compatibility**: No encoding issues in diffs, patches, or blame output

---

## Article II: The TDD Mandate

### §2.1 The Iron Law
Every significant change MUST be verified by at least one of:
- `./build.sh` compilation passes
- `bash e2e/trios_e2e_flow.sh` health/screenshot/log check passes
- Manual UI anomaly checklist verification (screenshot review)

**No exceptions.** A change without verification is a draft.

### §2.2 Test Format (trios)
```
Build check: swiftc compiles all .swift files without errors
E2E check: server health OK, app running, screenshot captured, no critical logs
UI checklist: title bar, tabs, input, glassmorphism, scroll, no duplicates
```

---

## Article III: No Prototype Mode

### §3.1 The Ban on Temporary Code
Trios **does not have** a prototype mode. There is no "I'll add tests later" grace period.

If you write a change without running `./build.sh`, the CI **will reject it**.

### §3.2 The Rationale
Verification written after implementation is not verification — it is retroactive justification. True TDD requires the check to exist **before** the merge, serving as:
1. A contract between agent and codebase
2. Executable documentation
3. A guard against regression
4. A design tool (the test tells you what the UI should do)

---

## Article IV: Validation Requirements

### §4.1 Build-Time Enforcement
The **build script** (`build.sh`) MUST:
1. Compile all `.swift` files with `swiftc`
2. Fail on any compilation error
3. Produce runnable `trios_app` binary

### §4.2 E2E-Level Enforcement
The **e2e flow** (`e2e/trios_e2e_flow.sh`) MUST:
1. Check server health at `127.0.0.1:9105/health`
2. Verify app process is running
3. Capture screenshot for manual anomaly review
4. Scan logs for critical errors

### §4.3 Run-Time Enforcement
All changes MUST be tested in the running app before land:
- Open panel (Cmd+Shift+T)
- Verify feature works (type, click, scroll)
- Verify no regressions in other tabs

---

## Article V: Amendment Process

### §5.1 What Can Be Amended
This constitution may be amended by:
1. Opening a GitHub issue with `constitution` label
2. Documenting the proposed change with full rationale
3. Obtaining **unanimous consent** from all architectural stewards

### §5.2 What Cannot Be Amended
The following are **immutable** and may never be changed:
- The Language Policy (Article I)
- The TDD Mandate (Article II)
- The Ban on Prototype Mode (Article III)
- The Validation Requirements (Article IV)

---

## Article VI: Enforcement

### §6.1 Agent Compliance
All AI agents working on trios MUST:
1. Check for test/build verification before creating new code
2. Reject changes without build pass with a hard error
3. Add verification steps when retrofitting existing code
4. Never bypass or disable validation

### §6.2 Human Compliance
Human contributors MUST:
1. Review build status in every PR
2. Request e2e pass for any change lacking it
3. Treat build failures as blocking issues

### §6.3 Automated Enforcement
The CI/CD pipeline MUST:
1. Run `./build.sh` on every commit
2. Block PRs where build fails
3. Block PRs where e2e flow fails

---

## Article VII: Sacred Trinity

Trios rests on three pillars. Violating any violates the whole:

1. **φ² + 1/φ² = 3** — The mathematical foundation (visible in UI via GoldenFloat)
2. **Swift macOS A2A Agent Network** — The computational substrate
3. **TDD-Inside-Build** — The verification mechanism (`build.sh` + e2e)

Additionally, the **Language Policy** (Article I) ensures universality and clarity.

---

## Article VIII: NO-NEW-SHELL (Toolchain Hygiene)

### §8.1 Statement
**No new Bourne-shell (`*.sh`) scripts** for validation, code generation, conformance, or data processing on the engineering critical path. Shell lacks static types, robust error semantics, and unit-test culture.

### §8.2 Permitted exceptions
1. **`build.sh`** — the canonical build script (swiftc compilation)
2. **`e2e/trios_e2e_flow.sh`** — the canonical e2e health check
3. **`scripts/setup-git-hooks.sh`** — one-time local bootstrap

### §8.3 Rationale
Aligns the repository with TDD-MANDATE: behavior lives in Swift code + build script, not in untested bash. Reduces macOS/Linux drift.

---

## Appendix: Quick Reference

| Command | Action |
|---------|--------|
| `./build.sh` | Compile all Swift sources |
| `bash e2e/trios_e2e_flow.sh` | Run e2e health check |
| `curl -s http://127.0.0.1:9105/health` | Check BrowserOS MCP server |
| `cat .trinity/SOUL.md` | Display this document |

---

**Enacted**: 2026-05-28
**Version**: 1.0 (trios adaptation)
**Status**: Immutable core (Articles I–IV per Article V)
