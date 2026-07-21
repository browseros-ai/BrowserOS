# ASCII purity specification for agents and skills

## Scope
Extend L3 PURITY from source code to all `.claude/agents/*.md` and `.claude/skills/*/*.md` files. These files are load-bearing policy for the T27 agent lattice and must be ASCII-only so every agent reads identical byte sequences regardless of editor or locale.

## Invariants
1. No `.claude/agents/*.md` line may contain a codepoint > 127.
2. No `.claude/skills/*/*.md` line may contain a codepoint > 127.
3. Specs, policy, and build files in `.trinity/specs/`, `.trinity/policy/`, `build.sh`, and Rust sources are already covered by earlier ASCII cleanup specs.
4. Replacements must preserve semantic meaning:
   - arrows `->` remain as `->`
   - em/en dashes `-`
   - bullets `- ` or `* `
   - emoji become semantic `[Label]` tags
   - Greek letters (`phi`) and superscripts (`^2`) become ASCII math
   - unknown codepoints become `[U+XXXX]` references
5. A lint skill must itself be ASCII-only; examples must use Unicode codepoint labels, not literal non-ASCII characters.

## Interface
No public API changes.

## Tests
- `grep -RIn '[^\x00-\x7F]' .claude/agents .claude/skills` returns zero violations.
- `./build.sh` passes after cleanup.
- `cargo test --workspace` passes after cleanup.

## Change flow
Apply in bulk via a one-off script that maps known non-ASCII codepoints to ASCII equivalents. Review the diff for semantic damage before committing. Update `ascii-lint/SKILL.md` with any newly seen characters.
