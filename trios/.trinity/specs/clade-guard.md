# CladeGuard rollback specification

## Scope
Define the rollback behavior of `BR-OUTPUT/CladeGuard.swift` so that restoring a Sovereign binary is atomic and only verified snapshots are used.

## Invariants
1. A rollback must never leave the target path without a runnable binary.
2. A snapshot without a matching `.sha256` checksum file must be treated as untrusted and rejected.
3. The running process must never be killed or restarted by CladeGuard (manual restart only).
4. Snapshot directory must be inside `ProjectPaths.trinity` so paths stay project-relative.

## Interface
- `snapshotCurrentBinary()` - copies the current runnable binary into the snapshot dir and writes a `.sha256` sidecar.
- `verifyChecksum(_ snapshotPath: String) -> Bool` - returns `true` only if the sidecar exists and matches the computed SHA-256.
- `applySnapshot(_ snapshotPath: String)` - atomically replaces each target binary via a temporary file and `FileManager.replaceItemAt`.

## Failure modes
- Missing snapshot dir: log and require manual intervention.
- Missing or mismatched checksum: log and reject the snapshot.
- Atomic replacement failure: leave the original binary untouched and log the error.

## Tests
- Unit test `verifyChecksum_missingSidecar_returnsFalse`.
- Unit test `verifyChecksum_mismatch_returnsFalse`.
- Unit test `verifyChecksum_matchingHash_returnsTrue`.

## Change flow
All changes to this file must be justified by this spec. Emergency hand edits require an `// AGENT-V-WAIVER:` block.
