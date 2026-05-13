# Task 3: Widen sessionId from `'main'` literal to `string`

## Summary

Widened `sessionId` from the `'main'` string literal type to `string` across 7 files (18 insertions, 18 deletions). All defaults remain `'main'` for backward compatibility. No logic changes.

## Files Modified

| File | Change |
|------|--------|
| `lib/agents/types.ts` | Widened `sessionId` in `AgentHistoryPage`, `AgentPromptInput`, `AgentRuntime` interface (getHistory, cancel, getRowSnapshot) |
| `lib/agents/agent-types.ts` | Widened `sessionId` in `AgentHistoryEntry` |
| `lib/agents/active-turn-registry.ts` | Widened `sessionId` in `ActiveTurnInfo`, `ActiveTurn`, `register()`, `getActiveFor()` |
| `lib/agents/acpx-runtime.ts` | Widened `sessionId` in `getHistory()`, `getRowSnapshot()`, `mapAcpxSessionRecordToHistory()`, `mapAgentMessageToHistoryEntry()` |
| `lib/agents/acpx-runtime-state.ts` | Widened `sessionId` in `LatestRuntimeState`, `deriveRuntimeSessionKey()` |
| `lib/agents/acpx-agent-adapter.ts` | Widened `sessionId` in `PrepareAcpxAgentContextInput` |
| `api/services/agents/agent-harness-service.ts` | Widened `sessionId` in `getActiveTurn()` |

## Testing

- All 321 passing tests continue to pass
- 117 pre-existing failures (all due to missing `@browseros/shared/*`, `hono`, `drizzle-orm`, `acpx/runtime` modules — unrelated dependency resolution issues)
- Zero regressions introduced by this change

## Commit

```
f40a9b2b refactor: widen sessionId from literal main to string
```
