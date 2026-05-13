# Task 4: Session CRUD REST Routes — Complete

## Summary

Implemented session CRUD routes for both ACP agents and assistant sessions following TDD.

## Files Created

| File | Purpose |
|------|---------|
| `apps/server/src/api/routes/agent-sessions.ts` | ACP agent session CRUD (5 endpoints) |
| `apps/server/src/api/routes/assistant-sessions.ts` | Assistant session stub (501 for all) |
| `apps/server/tests/api/routes/agent-sessions.test.ts` | 13 agent session route tests |
| `apps/server/tests/api/routes/assistant-sessions.test.ts` | 5 assistant session stub tests |

## Files Modified

| File | Change |
|------|--------|
| `apps/server/src/api/routes/agents.ts` | `/:agentId/sessions/main/history` → `/:agentId/sessions/:sessionId/history` |
| `apps/server/src/api/server.ts` | Import + register `createAgentSessionRoutes` + `createAssistantSessionRoutes` |

## Routes

### Agent Sessions (`/agents/:agentId/sessions`)
- `POST /` — create session (generates UUID, accepts optional `cwd`)
- `GET /` — list sessions (query: `search`, `cursor`, `limit`)
- `GET /:sessionId` — get session
- `PATCH /:sessionId` — update metadata (title, turnCount, lastMessagePreview, mode, model, meta)
- `DELETE /:sessionId` — close session (ref-count decrement)

### Assistant Sessions (`/assistant/sessions`)
- All 5 CRUD endpoints return `501 Not Implemented` (store doesn't exist yet)

## Test Results

```
18 pass, 0 fail, 33 expect() calls
```

## Notes

- Pre-existing issue: `agent-harness-service.ts` has wrong relative import for `AgentSessionStore` (`../../agent/` should be `../../../agent/`). This was not introduced by this task.
- Commit: `a4ca55f0` on `feat/session-backend`
