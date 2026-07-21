---
name: ascii-lint
description: Keep trios source, specs, agents, and skills ASCII-only per L3 PURITY.
argument-hint: [path]
---

# ASCII Lint Skill (trios)

Ensures all trios source, build, policy, agent, and skill files stay ASCII-only.

## When to Invoke

- Before sealing any wave.
- After bulk-editing agents or skills.
- When CI or t27-verifier reports non-ASCII characters.

## Scan

```bash
grep -RIn '[^\x00-\x7F]' {path}
```

A clean run returns no output.

## Safe Replacements

| Codepoint | Name | ASCII replacement |
| --- | --- | --- |
| U+2192 | rightwards arrow | `->` |
| U+2014 | em dash | `-` |
| U+2013 | en dash | `-` |
| U+2022 | bullet | `- ` or `* ` |
| U+00B7 | middle dot | ` / ` |
| U+2705 | white heavy check mark | `[OK]` |
| U+274C | cross mark | `[FAIL]` |
| U+26A0 | warning sign | `[WARN]` |
| U+1F451 | crown | `[Q]` |
| U+03C6 | greek small letter phi | `phi` |
| U+00B2 | superscript two | `^2` |
| U+00B3 | superscript three | `^3` |
| U+2082 | subscript two | `_2` |
| U+207B | superscript minus | `^-` |
| U+00F6 | latin small letter o with diaeresis | `oe` |
| U+FE0F | variation selector-16 | `` |
| emoji | any pictograph | semantic `[Label]` |

For unknown codepoints, use `[U+XXXX]` so the location is preserved and searchable.

## Automated Cleanup Script

```python
import os

TABLE = {
    "\u2192": "->",
    "\u2014": "-",
    "\u2013": "-",
    "\u2022": "- ",
    "\u00b7": " / ",
    "\u2705": "[OK]",
    "\u274c": "[FAIL]",
    "\u26a0": "[WARN]",
    "\U0001f451": "[Q]",
    "\u03c6": "phi",
    "\u00b2": "^2",
    "\u00b3": "^3",
    "\u2082": "_2",
    "\u207b": "^-",
    "\u00f6": "oe",
    "\ufe0f": "",
}

def clean(path):
    with open(path, "r", encoding="utf-8") as f:
        text = f.read()
    out = "".join(TABLE.get(c, f"[U+{ord(c):04X}]" if ord(c) > 0x7F else c) for c in text)
    if out != text:
        with open(path, "w", encoding="utf-8") as f:
            f.write(out)
        return True
    return False
```

## Rules

- Run cleanup only on text files (`*.md`, `*.swift`, `*.rs`, `*.sh`, `*.toml`).
- Never apply to binary assets, images, or `.plist` XML.
- Preserve semantic meaning; do not strip meaning just to pass the lint.
- After automated cleanup, run `grep -RIn '[^\x00-\x7F]'` again to confirm zero violations.
- Add new mappings to this skill when an unseen character appears.
