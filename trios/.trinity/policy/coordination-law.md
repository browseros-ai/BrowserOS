# Trinity Coordination Law (trios adaptation)

Before any task, every agent must read `.trinity` Akashic Chronicle, inspect active claims, queue, and swarm state, then acquire an exclusive claim on its target spec_path or graph_node. No mutation without prior read + claim.

This document is ported from `/Users/playra/t27/.trinity/policy/coordination-law.md` and adapted for the trios Swift/Rust codebase.

## Canonical Trinity Structure (trios)

```
.trinity/
├── events/              # Immutable append-only journal
│   └── akashic-log.jsonl
├── claims/              # Temporary ownership
│   ├── active/          # Current active claims
│   └── released/        # Historical claims
├── queue/               # Task management
│   ├── pending.json
│   ├── active.json
│   ├── blocked.json
│   └── done.json
├── experience/          # Learned memory
│   ├── episodes.jsonl
│   └── ring-{NNN}.md
├── state/               # Derived current reality
│   ├── queen-health.json
│   ├── swarm-health.json
│   └── ownership-index.json
├── specs/               # T27 behavior specs for canon Swift
│   └── *.md
└── policy/              # Coordination rules
    └── coordination-law.md
```

## Agent Startup Protocol

Every agent must execute this sequence before starting any task:

1. **Read Chronicle**: Append-read `.trinity/events/akashic-log.jsonl` for recent events.
2. **Inspect Claims**: Read `.trinity/claims/active/` to see what's claimed.
3. **Check Queue**: Read `.trinity/queue/` to find pending tasks.
4. **Verify Health**: Read `.trinity/state/queen-health.json` and `.trinity/state/swarm-health.json`.
5. **Acquire Claim**: Create exclusive claim on target resource with TTL.
6. **Record Intent**: Append task.intent event to events.
7. **Begin Mutation**: Only after claim is active.

## Task Intent Protocol

Before any mutation, agent must:

1. **Check**: Is task_id already in `.trinity/queue/active.json`?
2. **If claimed**: Do NOT work on it. Options:
   - Wait for release
   - Pick different task_id
   - Request handoff (if owner is stuck)
3. **If not claimed**: Proceed with claim acquisition.

## Claim Protocol

### Acquire Claim

Agents SHOULD implement claim logic via Swift/Rust or MCP tools, not new shell scripts (L7 UNITY). The canonical JSON shape:

```json
{
  "claim_id": "uuid",
  "agent_id": "agent-name",
  "spec_path": "relative/path/to/spec.md",
  "graph_node": "recursion_guard",
  "task_id": "TASK-001",
  "acquired_at": "2026-07-21T10:00:00Z",
  "ttl_sec": 1800,
  "expires_at": "2026-07-21T10:30:00Z",
  "heartbeat_at": "2026-07-21T10:00:00Z",
  "priority": "P0"
}
```

### Heartbeat Protocol

Every agent with active claims must heartbeat every 60 seconds by updating `heartbeat_at` in the active claim file and appending a `claim.heartbeat` event.

### Release Claim

On completion:

1. Move `.trinity/claims/active/{resource}.json` to `.trinity/claims/released/{claim_id}.json`.
2. Append `claim.release` event with result `clean`, `toxic`, or `cancelled`.
3. Update queue: move task from active to done on `clean`, or to blocked on `toxic`.

## Conflict Prevention

### Resource Ownership Rules

- One writable owner per resource (spec_path or graph_node).
- Other agents are read-only on claimed resources.
- Claims have TTL; stale claims can be reclaimed after timeout.

### Priority Rules (trios)

| Priority | Domain | TTL | Reclaim Wait |
|----------|--------|-----|---------------|
| P0 (Critical) | QueenLotus, RecursionGuard, MenuBarLogo | 30 min | 5 min |
| P1 (High) | MCP/Bridge, A2A, ChatLogic | 60 min | 15 min |
| P2 (Normal) | UI/UX, Git, Terminal, Settings | 120 min | 30 min |

### Phi-Critical and Sacred-Core

For `phi_critical` and `sacred_core` nodes (menu-bar logo, GoldenFloat, L5 constants):

- Stricter locking required.
- Preferred verifier handoff.
- Claims persist longer (no auto-reclaim).
- Manual approval for release.

## Event Logging

All events are append-only to `.trinity/events/akashic-log.jsonl`:

```jsonl
{"ts":"2026-07-21T10:00:00Z","event":"task.intent","agent":"t27-creator","task_id":"RECURSION-001","spec_path":".trinity/specs/recursion-guard.md","graph_node":"recursion_guard","priority":"P0"}
{"ts":"2026-07-21T10:00:01Z","event":"claim.acquire","agent":"t27-creator","claim_id":"claim-001","resource":".trinity/specs/recursion-guard.md","ttl_sec":1800}
{"ts":"2026-07-21T10:05:00Z","event":"claim.release","agent":"t27-creator","claim_id":"claim-001","result":"clean"}
```

## State Materialization

`.trinity/state/` contains derived current reality:

| File | Source | Refresh Policy |
|------|--------|----------------|
| `queen-health.json` | Derived from health signals + events | Every minute |
| `swarm-health.json` | Aggregated from all domains | Every minute |
| `ownership-index.json` | Active claims index | Every 30 seconds |

## Short Laws

**Before any task, every agent must read .trinity Akashic Chronicle, inspect active claims, queue, and swarm state, then acquire an exclusive claim on its target spec_path or graph_node. No mutation without prior read + claim.**

**No agent has rights to write to spec_path or graph_node without active claim.**

**One writable owner per resource; all others read-only. This ownership boundary is foundational production-practice for multi-agent coordination.**

**Claim has TTL and heartbeat; if agent dies, claim can be reclaimed after timeout.**

**For phi-critical and sacred-core nodes, stricter lock required; preferably verifier handoff.**

**Experience does not replace event log: events = immutable journal, experience = learned interpretation, state = derived current view.**

## Loop Handoff Protocol

Each PHI LOOP must end with a `loop.handoff` event containing **three future options**. The next loop reads these options and chooses one.

### Output Format

```text
[FUTURE OPTIONS]
  1) OPTION-ID — description
  2) OPTION-ID — description
  3) OPTION-ID — description
```

### Event Format

```json
{
  "ts": "2026-07-21T10:30:00Z",
  "event": "loop.handoff",
  "loop_id": "2026-07-21T10:30",
  "agent_id": "t27-queen",
  "trace_id": "uuid",
  "past": {"summary": "sealed RecursionGuard spec"},
  "present": {"summary": "ready to migrate ChatLogic"},
  "future_options": [
    {"id": "chatlogic-spec", "label": "Spec-drive ChatLogic and extend unit tests", "priority": "P0", "domain": "Chat"},
    {"id": "mcp-client-spec", "label": "Spec-drive TriosMCPClient error handling", "priority": "P1", "domain": "MCP"},
    {"id": "ui-h-spec", "label": "Assign H agent to glassmorphism theme spec", "priority": "P2", "domain": "UI"}
  ],
  "chosen_option": null
}
```

### No FUTURE = Toxic Completion

If a loop ends without `[FUTURE OPTIONS]` and at least 3 options, the loop is marked **drifted**. The next loop must read the last successful handoff or ask the user for a new 3-option plan.
