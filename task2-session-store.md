# Task 2: AgentSessionStore — Complete

## Summary

Implemented `AgentSessionStore` with ref-counted session metadata and listing, following TDD.

## Files Created

| File | Lines | Purpose |
|------|-------|---------|
| `src/agent/agent-session-store.ts` | ~120 | In-memory session metadata store with ref-counting |
| `tests/agent/agent-session-store.test.ts` | ~245 | 18 tests covering all API surface |

## API Implemented

```ts
class AgentSessionStore {
  openSession(agentId, sessionId, cwd?) → ActiveSession   // ref-count on re-open
  closeSession(sessionId) → number                         // -1 if unknown, 0 = deleted
  listSessions(agentId, {cursor?, limit?, search?}) → ActiveSession[]
  getSessionMeta(sessionId) → ActiveSession | null
  updateSessionMeta(sessionId, updates) → ActiveSession | null
}
```

## Test Results

```
18 pass, 0 fail, 56 expect() calls
```

### Test Coverage
1. ✅ Opens a new session and tracks it
2. ✅ Increments ref count on re-open
3. ✅ Decrements ref count on close; deletes at zero
4. ✅ Lists sessions for an agent ordered by updatedAt
5. ✅ Searches sessions by title (case-insensitive)
6. ✅ Supports cursor-based pagination
7. ✅ Updates session metadata
8. ✅ Returns null for unknown session operations

## TDD Process
- **RED**: Test written first, verified failure (module not found)
- **GREEN**: Implementation written, 15/18 passed initially (timing issues with same-ms `updatedAt`)
- **Fix**: Added `Bun.sleep(1)` between session creations for stable sort ordering
- **Lint**: Removed unused `ActiveSession` type import, replaced `!` non-null assertions with `?.`

## Design Decisions
- **No logger dependency**: The `@browseros/shared` module resolution fails in test context (known monorepo issue). The store is a pure in-memory data structure; logging can be added at the integration layer.
- **Auto `lastMessageAt`**: When `lastMessagePreview` is updated without explicit `lastMessageAt`, timestamp is auto-set.
- **Ref-count returns -1** for unknown sessions (distinct from "deleted" which returns 0).

## Commit
```
68a365a9 feat(agent): add AgentSessionStore with ref-counting and listing
```
