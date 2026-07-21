# QueenStatusViewModel shell-free specification

## Scope
Remove all `/bin/zsh -c` invocations from `BR-OUTPUT/QueenStatusViewModel.swift` and replace them with tokenized `Process()` calls using absolute executables and discrete argument arrays.

## Invariants
1. No function in `QueenStatusViewModel` may invoke `/bin/zsh` or any other shell with user-influenced text.
2. Health probes must use absolute executable paths (`/usr/bin/curl`, `/usr/bin/git`, `/usr/bin/pgrep`, `/bin/ps`, `/usr/bin/swiftc`).
3. Arguments must be passed as an array; no string interpolation that could alter argv boundaries.
4. Directory counts and log tail reads must use `FileManager` instead of `ls`, `wc`, or `tail`.

## Interface
Add private helpers:
- `run(_ executable: String, arguments: [String], workDir: String?) -> String` - synchronous tokenized Process.
- `runAsync(_ executable: String, arguments: [String], workDir: String?) async -> String` - detached wrapper.

Replace:
- `shellAsync("curl ...")` -> `runAsync("/usr/bin/curl", arguments: [...])`
- `shellAsync("pgrep ...")` -> `runAsync("/usr/bin/pgrep", arguments: [...])`
- `shellAsync("git ...")` -> `runAsync("/usr/bin/git", arguments: [...])`
- `shellAsync("swiftc ...")` -> `runAsync("/usr/bin/swiftc", arguments: [...])`
- `shell("tail -n 20 ...")` -> `FileManager` read + suffix(20)
- `shell("ls ... | wc -l")` -> `FileManager` directory enumeration

## Failure modes
- If the executable is missing, the helper returns an empty string and logs the error.
- Health probes treat empty/unexpected output as unhealthy.

## Tests
- `./build.sh` passes.
- `grep` for `shellAsync`, `shell(`, `/bin/zsh` in `QueenStatusViewModel.swift` returns zero matches.

## Change flow
All changes must be justified by this spec. Emergency hand edits require an `// AGENT-V-WAIVER:` block.
