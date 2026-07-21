# ASCII-only cleanup specification

## Scope
Remove all non-ASCII characters from `BR-OUTPUT/*.swift` to comply with trios L3 PURITY (ASCII-only source).

## Invariants
1. No `BR-OUTPUT/*.swift` source line may contain a codepoint > 127.
2. Replacements must preserve semantic meaning:
   - `- ` bullet -> `- ` or `* `
   - `-` em-dash -> `-`
   - `-` en-dash -> `-`
   - ` / ` middle dot -> ` / `
   - `->` arrow -> `->`
   - `[WARN]` warning emoji -> `[WARN]`
   - `[Q]` crown -> `[Q]`
3. UI strings must remain readable; use ASCII labels or SF Symbols if needed.

## Interface
No public API changes. Internal string literals and comments only.

## Tests
- Python ASCII scan over `BR-OUTPUT/*.swift` returns zero violations.
- `./build.sh` passes after cleanup.

## Change flow
All changes must be justified by this spec. Emergency hand edits require an `// AGENT-V-WAIVER:` block.
