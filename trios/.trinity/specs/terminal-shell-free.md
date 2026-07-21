# Terminal shell-free execution specification

## Scope
Remove all `/bin/zsh -c` and string interpolation from `BR-OUTPUT/TerminalTabView.swift` so user-supplied commands are dispatched as tokenized `Process()` invocations with a strict executable allowlist.

## Invariants
1. No shell interpreter is spawned for any terminal command.
2. Commands are parsed into `{executable, arguments}` by `TerminalCommandSanitizer.sanitize()`.
3. Only absolute or allowlisted executables run (`/bin/ls`, `/usr/bin/env`, `swiftc`, etc.).
4. Forbidden tokens (`;`, `|`, `&&`, `$()`, backticks, redirections) are rejected before execution.
5. Output streams are captured and surfaced in the UI; errors include the failing executable path.
6. L3 PURITY: no non-ASCII characters in source or log strings.

## Interface
```swift
struct TerminalRequest {
    let executable: String
    let arguments: [String]
}

enum TerminalCommandError: Error {
    case emptyCommand
    case forbiddenToken(String)
    case executableNotAllowed(String)
    case missingExecutable
}

struct TerminalCommandSanitizer {
    static func sanitize(_ command: String) throws -> TerminalRequest
}
```

## Algorithm
1. Trim whitespace; reject empty input.
2. Split on ASCII space into tokens.
3. Map first token to executable:
   - If absolute path, keep it.
   - Else look up in `allowedBaseNames` set.
4. Reject tokens that contain `;`, `|`, `&`, `$`, backtick, `<`, `>`, `(`, `)`, `\`, newline.
5. Return `TerminalRequest`.
6. Execute with `Process(executableURL: URL(fileURLWithPath:), arguments:)` and capture stdout/stderr.
7. On failure, log `[FAIL] terminal: {executable} -> {error}`.

## Tests
- `./build.sh` passes.
- `swiftc` compiles `BR-OUTPUT/TerminalTabView.swift` without warnings.
- Sanitizer unit tests (optional Swift unit) for:
  - simple `ls -la`
  - rejected `rm -rf /`
  - rejected `foo; bar`
  - rejected backtick command substitution

## Change flow
Spec-first via `t27-creator` or hand-edit with `// AGENT-V-WAIVER:` block. Land only after `t27-verifier` confirms no `zsh`/`bash`/`sh` subprocess strings remain.
