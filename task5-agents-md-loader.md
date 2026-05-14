# Task 5: AGENTS.md Loader with Allowlist + Cache

## Status: ✅ DONE

## Files Created

| File | Purpose |
|------|---------|
| `apps/server/src/sessions/agents-md-loader.ts` | `AgentsMdLoader` class with allowlist validation, path traversal protection, 100KB size limit, and mtime-based cache |
| `apps/server/tests/sessions/agents-md-loader.test.ts` | 7 test cases covering all requirements |

## Test Results

```
7 pass | 0 fail | 12 expect() calls | 81ms
```

### Test Cases

1. ✅ Loads AGENTS.md from allowed workspace path
2. ✅ Returns null for disallowed path (not in allowlist)
3. ✅ Returns null when AGENTS.md does not exist
4. ✅ Rejects files exceeding 100KB size limit
5. ✅ Caches and detects changes via mtime (stat check)
6. ✅ Loads multiple workspace AGENTS.md files
7. ✅ Prevents path traversal (`../../etc/passwd`)

## Implementation Details

- **Allowlist**: Constructor takes `registeredPaths[]`, all resolved via `path.resolve()`
- **Path traversal guard**: `isAllowed()` checks resolved path starts with an allowed dir + separator
- **Size limit**: `stat().size > 100KB` → returns null
- **Cache**: `Map<string, CacheEntry>` keyed by resolved AGENTS.md path; invalidated when mtime changes
- **`loadMultiple()`**: Parallel load, filters nulls

## Commit

```
a391d652 feat(sessions): add AGENTS.md loader with allowlist and cache
```
