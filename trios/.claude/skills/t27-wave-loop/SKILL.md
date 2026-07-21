---
name: t27-wave-loop
description: T27 standing-wave charter for trios — explore a problem, generate 2-4 variants, implement one, closeout with 3 future options.
argument-hint: [domain] [context]
---

# T27 Wave Loop Skill (trios adaptation)

Adapted from `/Users/playra/t27/.claude/skills/t27-wave-loop.md`. Use for larger refactoring waves where multiple implementation variants exist.

## Charter

1. **Explore** — read specs, experience, current code.
2. **Variant** — generate 2–4 concrete implementation options (Road A/B/C + one experimental).
3. **Decide** — t27-queen picks one variant based on priority, risk, and domain balance.
4. **Implement** — run `/t27-phi-loop` on the chosen variant.
5. **Closeout** — seal, verify, save experience, and produce `[FUTURE OPTIONS]`.

## Output Format

At decision point:

```
## T27 Wave Loop — Variants
Domain: {domain}
Context: {context}

Option 1 (Conservative / Road A):
{description, risk, effort}

Option 2 (Balanced / Road B):
{description, risk, effort}

Option 3 (Deep / Road C):
{description, risk, effort}

Option 4 (Experimental):
{description, risk, effort}

Recommended: Option {N}
```

At closeout:

```
## T27 Wave Loop — Closeout
Chosen: Option {N}
Status: {SEALED|DRIFTED|TOXIC}
Artifacts: {list}
Experience: {path}

[FUTURE OPTIONS]
  1) {next wave option 1}
  2) {next wave option 2}
  3) {next wave option 3}
```

## Rules

- Always produce at least 3 future options at closeout.
- Save experience after every wave.
- Experimental variants must not break Sovereign; use Canary worktree.
