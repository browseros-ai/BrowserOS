---
description: Save learning and experience to persistent memory for trios
description: Captures learnings from trios development for future reference and agent improvement.
parameters:
  - name: ring
    type: string
    description: Ring or area where learning occurred (e.g., "SR-02", "BR-OUTPUT", "A2A")
  - name: phase
    type: string
    description: Phase where learning occurred (issue, spec, tdd, impl, seal, verify, land, learn)
  - name: insight
    type: string
    description: The learning or insight to save
---

# Experience Save Skill (trios)

Captures learnings from trios macOS app development for future reference and agent improvement.

## What to Save

- Debugging insights and solutions
- UI/UX pattern discoveries (SwiftUI tricks, AppKit bridging)
- macOS-specific gotchas (firstResponder, NSPanel, NSTextView)
- A2A protocol learnings (SSE parsing, state machine races)
- Build system clarifications (swiftc flags, module ordering)
- L3/L4/L7 law clarifications
- Anti-patterns to avoid

## Storage Locations

Learnings are saved to:
- `.trinity/experience.md` - General learnings (append)
- `.trinity/experience/YYYY-MM-DD_{topic}.json` - Structured episode
- `.trinity/experience/mistakes-catalog.json` - Mistake registry

## Episode JSON Schema

```json
{
  "id": "uuid-or-hash",
  "date": "YYYY-MM-DD",
  "ring": "SR-02",
  "phase": "verify",
  "road": "A|B|C",
  "agents_involved": ["T", "H", "K"],
  "issue": "#N or description",
  "problem": "What went wrong",
  "root_cause": "Why it happened",
  "fix": "What was changed",
  "files_changed": ["path1", "path2"],
  "verification": "build + e2e results",
  "pattern": "Reusable insight",
  "anti_pattern": "What to avoid next time",
  "tags": ["firstResponder", "NSTextView", "SSE", "state_machine"]
}
```

## Format (Markdown appendix)

```markdown
## YYYY-MM-DD - {Brief Title}

**Ring:** {SR-NNN or BR-OUTPUT}
**Phase:** {phase}
**Road:** {A|B|C}
**Agents:** {letters}

### Problem
{Description}

### Root Cause
{Analysis}

### Fix
{What changed}

### Pattern
{Reusable insight}

### Anti-pattern
{What to avoid}
```

## Access

Saved learnings are:
- Automatically loaded in subsequent sessions via `Read .trinity/experience.md`
- Used for pattern matching when planning (MNL: Mistake -> Not-repeat -> Learning)
- Incorporated into agent decision-making

## Usage

Call this skill when:
- Completing the "Learn" phase of PHI LOOP
- Discovering a useful pattern during implementation
- Solving a non-trivial bug (especially macOS/SwiftUI quirks)
- Finding a better approach than initially planned
- Avoiding a repeated mistake
