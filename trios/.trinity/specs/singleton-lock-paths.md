# Singleton lock path specification

## Scope
Move the trios singleton lock and PID files from world-writable `/tmp` to a project-relative, user-private runtime directory.

## Invariants
1. The lock file must live under `.trinity/run/` inside the project root.
2. The runtime directory must be created with `0o700` permissions.
3. The lock file itself must be created with `0o600` permissions.
4. The PID file must be written atomically and removed on graceful exit.

## Interface
- `ProjectPaths.trinityRun` -> `"\(trinity)/run"`
- `ProjectPaths.singletonLockFile` -> `"\(trinityRun)/trios_singleton.lock"`
- `ProjectPaths.singletonPIDFile` -> `"\(trinityRun)/trios_singleton.pid"`

## Failure modes
- If the runtime directory cannot be created, log the error and fall back to the open attempt (which will fail closed if permissions are wrong).
- Stale PID files must still be detectable and removable.

## Tests
- `./build.sh` passes.
- Manual test: launch trios twice; second instance activates the first and exits.
- `ls -ld .trinity/run` shows `drwx------` after first launch.

## Change flow
All changes must be justified by this spec. Emergency hand edits require an `// AGENT-V-WAIVER:` block.
