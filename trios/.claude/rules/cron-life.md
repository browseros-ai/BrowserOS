# Queen Cron Life - Autonomous Agent Lifecycle (Clade-aware v2)

## Schedule
| Interval | Task | Purpose |
|----------|------|---------|
| Every 15 min | /tri quick | Health check Sovereign |
| Every 15 min | /clade-guard quick | Health check Sovereign + Canary |
| Every 30 min | /doctor quick | Build + dirty check |
| Every 60 min | /clade-seal audit | Full seal audit on Canary |
| Every 60 min | /god-mode status | Full oversight |
| Every 24h | /tri audit | Deep project audit |
| Every 24h | /clade-seal deep | Screenshot baseline + fitness sync |
| Every 24h | wrap-up | Session memory save |
| Every ~60s | trios app watchdog | Relaunch trios.app if its process is gone (keeps the menu-bar logo alive) |
| On boot | trios_app + bun server | Start services |

## Clade-Specific Triggers
- Sovereign health fail > 30s -> `cargo run --bin clade-rollback` auto-restore
- Canary build fail > 3 times -> mark clade `extinct`, reset worktree
- Safety budget <= 0 -> halt all auto-improvement, notify user
- Dirty files > 5 in worktree -> `/doctor commit` on canary branch
| **trios app process absent -> `clade-monitor` relaunches `open trios.app` within ~60s (INVARIANT: the menu-bar logo must always be present)** |
| Server down > 5 min -> restart pm2 |
| No wrap-up > 2h -> force wrap-up |
| Git branch = main -> create feature branch |

## INVARIANT: trios menu-bar logo must never disappear
The trios status-bar logo vanishes only when the **app process dies** (crash,
rebuild without restart, logout). Protections, in order:
1. **App watchdog** in `clade-monitor` (every ~60s): if no `trios`/`trios_app`
   process exists, relaunch via `open trios.app`. trios's `RecursionGuard`
   (POSIX flock) prevents duplicate instances.
2. **Icon fallback chain** in `main.swift:setupStatusItem()`: bundle `logo.png`
   -> `ProjectPaths.logoPNG` -> bundle-path logo -> generated colored circle. The
   status item is therefore *always* drawable when the app runs.
3. **Rebuild rule:** after `./build.sh` / `clade-build`, the running app keeps
   the OLD binary until restarted. Either relaunch it (`open trios.app`) or let
   the watchdog catch it - never leave the app killed after a rebuild.
- Launch via the **`.app` bundle** (`open trios.app`), not the bare `./trios_app`,
  so `Bundle.main` resolves the logo resource.

## Repositories Monitored
1. trios (current) - Swift macOS app + Rust rings
2. trinity - Zig core + agents
3. t27 - Language spec + skills
4. trios-mcp-rag - Rust MCP server
5. trinity-s3ai - Research formulas

## Wake Protocol
When triggered:
1. Check `.trinity/state/last_wake.json`
2. If > interval, proceed
3. Run appropriate skill (`/clade-guard`, `/clade-seal`, `/doctor`)
4. Log result to `.trinity/event_log.jsonl`
5. Update `last_wake.json` with timestamp + clade_id

## Sleep Protocol
After work:
1. Verify no critical violations (safety budget > 0, Sovereign healthy)
2. Log summary to `event_log.jsonl`
3. Update `.trinity/experience.md` if learning occurred
4. Sleep until next interval

## Rust Cron Monitor
```bash
cargo run --bin clade-monitor
```
Background daemon checking all intervals + triggers.
