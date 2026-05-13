# Session Management — Deferred Backend Work

Date: 2026-05-12
Status: DEFERRED
Mode: Backend / server / protocol / persistence
Reason: Current delivery scope is **frontend-only**. Backend cannot be modified in this phase.

---

## 1. Decision

All backend-impacting work discovered during ACP + Assistant session planning is deferred here.

Frontend specs may depend on these capabilities, but must:
- use existing backend behavior where possible
- degrade gracefully when backend support is missing
- avoid assuming new server endpoints exist

---

## 2. ACP Deferred Backend Work

### P0

| Item | Why deferred |
|------|--------------|
| Register ACP session REST routes in Hono (`GET/POST/DELETE /:agentId/sessions...`) | Requires server route changes |
| Implement ACP `load_session` backend flow | Requires harness/runtime replay logic |
| Implement ACP `resume_session` backend flow | Requires runtime/session lifecycle changes |
| Implement ACP close-session route | Requires server route registration |
| Implement ACP set-title route | Requires server route registration |
| Implement ACP session search on server | Requires API/query changes |

### P1

| Item | Why deferred |
|------|--------------|
| Capability negotiation (`supports_*`, session capabilities) | Requires protocol/server changes |
| Watch/push session list refresh channel | Requires server push/subscription support |
| ACP session workDir/project association | Requires ACP/server data model changes |
| Session modes/model/config backend support | Requires adapter/runtime support |

### P2/P3

| Item | Why deferred |
|------|--------------|
| ACP truncate/retry/title trait objects | Requires runtime operation support |
| ACP manual compaction trigger | Requires server compaction hook |
| ACP protocol debug log | Requires backend logging surface |
| `pendingLoads` async dedup | Requires async session-open implementation |

---

## 3. Assistant Deferred Backend Work

### P0

| Item | Why deferred |
|------|--------------|
| Backward-compat metadata migration from existing local conversations | Requires persistence/migration path |
| Persisted per-session `autoLoadAgentsMd` config | Requires session API/schema support |
| Proper workspace removal persistence (`removeWorkspace`) | Requires server session metadata mutation |

### P1

| Item | Why deferred |
|------|--------------|
| Session search API (`/assistant/sessions?search=`) | Requires backend filtering/search support |
| Server-driven pagination contract beyond current sources | Requires API integration path |
| Remote/GraphQL conversation workspace metadata | Requires backend schema/response changes |
| True compact execution behind `/assistant/sessions/:id/compact` | Current server path is stubbed |
| Tags/config persistence if missing in current frontend source path | Requires session API usage/support |

### P2

| Item | Why deferred |
|------|--------------|
| Server-backed title edit persistence everywhere | Requires unified persistence path |
| Server-backed session tags UX contract | Requires endpoint/data consistency |

---

## 4. Frontend Rules While Backend Is Deferred

### ACP frontend
- Treat multi-session as UI/state architecture only unless existing APIs already support it.
- Do not assume dedicated ACP session CRUD endpoints exist.
- If `X-Session-Id` is unsupported in a path, fall back to current single-session UX.
- Hide unsupported controls behind capability/availability checks.

### Assistant frontend
- Use existing conversation sources first.
- Render workspace grouping/bubbles from available local data.
- Do not require new GraphQL fields to function.
- Search/pagination UI must degrade gracefully if only local/client filtering is available.
- If compaction endpoint is stubbed, expose no-op or hide trigger until backend exists.

---

## 5. Exit Criteria For Un-defer

Backend work can move out of this deferred spec only when:
1. backend scope is explicitly approved
2. endpoint/schema ownership is clear
3. frontend fallback behavior is documented
4. verification plan covers both local and remote/session-backed flows
