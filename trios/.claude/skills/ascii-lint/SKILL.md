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
| U+2026 | horizontal ellipsis | `...` |
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
| U+2550 | box drawings double horizontal | `=` |
| U+2501 | box drawings heavy horizontal | `-` |
| U+2500 | box drawings light horizontal | `-` |
| U+2502 | box drawings light vertical | `|` |
| U+2503 | box drawings heavy vertical | `|` |
| U+251C | box drawings light vertical and right | `+` |
| U+2524 | box drawings light vertical and left | `+` |
| U+252C | box drawings light down and horizontal | `+` |
| U+2534 | box drawings light up and horizontal | `+` |
| U+253C | box drawings light vertical and horizontal | `+` |
| U+1F534 | large red circle | `[REJECT]` |
| U+1F7E2 | large green circle | `[PASS]` |
| U+1F7E1 | large yellow circle | `[WARN]` |
| U+1F4CD | round pushpin | `[LOC]` |
| U+1F4C1 | file folder | `[DIR]` |
| U+1F4BE | floppy disk | `[SAVE]` |
| U+1F4E6 | package | `[PKG]` |
| U+1F510 | closed lock with key | `[LOCK]` |
| U+1F6A8 | police cars revolving light | `[ALERT]` |
| U+1F4DC | scroll | `[DOC]` |
| U+1F4CA | bar chart | `[CHART]` |
| U+1F4DD | memo | `[NOTE]` |
| U+1F5D1 | wastebasket | `[BIN]` |
| U+23ED | black right-pointing double triangle with vertical bar | `[SKIP]` |
| U+23F3 | hourglass not done | `[WAIT]` |
| U+1F4B0 | money bag | `[COST]` |
| U+1F9EC | dna | `[DNA]` |
| emoji | any pictograph | semantic `[Label]` |

For unknown codepoints, use `[U+XXXX]` so the location is preserved and searchable.

## Automated Cleanup Script

Use Python with `chr()` so the script itself stays ASCII-only:

```python
import os

PAIRS = [
    (0x2192, "->"),
    (0x2014, "-"),
    (0x2013, "-"),
    (0x2022, "- "),
    (0x00B7, " / "),
    (0x2026, "..."),
    (0x2705, "[OK]"),
    (0x274C, "[FAIL]"),
    (0x26A0, "[WARN]"),
    (0x1F451, "[Q]"),
    (0x03C6, "phi"),
    (0x00B2, "^2"),
    (0x00B3, "^3"),
    (0x2082, "_2"),
    (0x207B, "^-"),
    (0x00F6, "oe"),
    (0xFE0F, ""),
    (0x2550, "="),
    (0x2501, "-"),
    (0x2500, "-"),
    (0x2502, "|"),
    (0x2503, "|"),
    (0x251C, "+"),
    (0x2524, "+"),
    (0x252C, "+"),
    (0x2534, "+"),
    (0x253C, "+"),
    (0x1F534, "[REJECT]"),
    (0x1F7E2, "[PASS]"),
    (0x1F7E1, "[WARN]"),
    (0x1F4CD, "[LOC]"),
    (0x1F4C1, "[DIR]"),
    (0x1F4BE, "[SAVE]"),
    (0x1F4E6, "[PKG]"),
    (0x1F510, "[LOCK]"),
    (0x1F6A8, "[ALERT]"),
    (0x1F4DC, "[DOC]"),
    (0x1F4CA, "[CHART]"),
    (0x1F4DD, "[NOTE]"),
    (0x1F5D1, "[BIN]"),
    (0x23ED, "[SKIP]"),
    (0x23F3, "[WAIT]"),
    (0x1F4B0, "[COST]"),
    (0x1F9EC, "[DNA]"),
]
TABLE = {chr(c): r for c, r in PAIRS}

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
- `Cargo.toml` `description` fields are source text and must also be ASCII-only.
