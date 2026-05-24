# Queen Cron Life — Autonomous Agent Lifecycle

## Schedule
| Interval | Task | Purpose |
|----------|------|---------|
| Every 15 min | /tri quick | Health check |
| Every 30 min | /doctor quick | Build + dirty check |
| Every 60 min | /god-mode status | Full oversight |
| Every 24h | /tri audit | Deep project audit |
| Every 24h | wrap-up | Session memory save |
| On boot | trios_app + bun server | Start services |

## Trigger Conditions
- Build broken → /doctor full (auto-fix)
- Dirty files > 5 → /doctor commit
- Server down > 5 min → restart pm2
- No wrap-up > 2h → force wrap-up
- Git branch = main → create feature branch

## Repositories Monitored
1. trios (current) — Swift macOS app
2. trinity — Zig core + agents
3. t27 — Language spec + skills
4. trios-mcp-rag — Rust MCP server
5. trinity-s3ai — Research formulas

## Wake Protocol
When triggered:
1. Check .trinity/state/last_wake.json
2. If > interval, proceed
3. Run appropriate skill
4. Log result to .trinity/event_log.jsonl
5. Update last_wake.json

## Sleep Protocol
After work:
1. Verify no critical violations
2. Log summary to event_log
3. Update experience.md if learning occurred
4. Sleep until next interval