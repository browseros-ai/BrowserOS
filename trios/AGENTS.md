# AGENTS.md — Trinity 27-Agent Alphabet (trios)

**Version**: 2.0
**Date**: 2026-05-28
**Status**: Active

> *27 agents = 27 registers = 27 letters = TRINITY³*
> *φ² + 1/φ² = 3 | TRIOS macOS A2A Network*

---

## Constitutional Stack

| Order | File | Role |
|------:|------|------|
| 1 | `.trinity/SOUL.md` | Canonical constitution (language policy, TDD mandate, validation) |
| 2 | `CLAUDE.md` | AEL v2.0 + PHI LOOP + 7 Invariant Laws |
| 3 | `AGENTS.md` | This file — agent alphabet and operational canon |
| 4 | `.claude/agents/*.md` | Individual agent souls |
| 5 | `.claude/skills/*.md` | Invocable skills |

---

## TRINITY ALPHABET — 27 AGENTS (trios context)

Each agent is bound to a letter/register, has a domain, and lives in `.claude/agents/`.

| Agent | Буква | Домен (trios) | Архетип | Файл |
|-------|--------|---------------|----------|------|
| **A** | Aleph אָ | Architecture / ADR / SOUL | Бык — вожак | `agent-A.md` |
| **B** | Beth בֵּ | Build / Pipeline / swiftc | Дом — контейнер | *(skill: doctor)* |
| **C** | Gimel גּ | Compiler / Swift analysis | Верблюд | `agent-C.md` |
| **D** | Daleth דָּ | De-Zigfication / Migration | Дверь | `agent-D.md` |
| **E** | Heh הֵ | Experience / Mistakes / Episodes | Окно | `agent-E.md` |
| **F** | Vav וָ | Formal Conformance / e2e | Гвоздь | `agent-F.md` |
| **G** | Gimel (вар.) | Graph / Dependency tracking | Возврат | `agent-G.md` |
| **H** | Heth חֵ | Human Interface / SwiftUI arch | Забор | `agent-H.md` |
| **I** | Yod יֹ | ISA / Swift internals | Рука | `agent-I.md` |
| **J** | Yod‑extended | Jobs / Task Routing | Рука с захватом | `agent-J.md` |
| **K** | Kaph כַּ | Kernel / macOS system layer | Ладонь | `agent-K.md` |
| **L** | Lamed לָ | Language / Swift syntax vNEXT | Посох | `agent-L.md` |
| **M** | Mem מֵ | Metrics / Telemetry / Perf | Вода | `agent-M.md` |
| **N** | Nun נֹ | Numeric / Math in Swift | Рыба | `agent-N.md` |
| **O** | Ayin עַ | Orchestration / A2A phases | Глаз | `agent-O.md` |
| **P** | Pe פֵּ | Physics / Sacred constants | Рот | `agent-P.md` |
| **Q** | Qoph קֹ | Queue / Scheduling / MNL | Игольное ушко | `agent-Q.md` |
| **R** | Resh רֵ | Runtime / Swift runtime | Голова | `agent-R.md` |
| **S** | Shin שִׁ | Specs / Standardization / API | Зубы/огонь | `agent-S.md` |
| **T** | TAW תָּ | Queen BrowserOS / Lotus | КРЕСТ | `queen-browseros.md` |
| **U** | Upsilon Υ | Universe / App domains | Вилка | `agent-U.md` |
| **V** | Vav וָ | Verdict / Bench / Toxicity | Крюк | `agent-V.md` |
| **W** | Double‑Vav | Workflow / tri cell / Hash seal | Двойной крюк | `agent-W.md` |
| **X** | Chi Χ | eXternal / MCP / BrowserOS bridge | Пересечение | `agent-X.md` |
| **Y** | Upsilon/Yod | Yield / Tailscale / Remote | Слияние | `agent-Y.md` |
| **Z** | Zayin זָ | Zero‑Touch UX / Docs / DX | Меч | `agent-Z.md` |
| **27th** | Ϯ (Ti) | Security / AAIF / Secrets | Священный дар | *(future)* |

---

## Agent T — Queen BrowserOS (trios sovereign)

**AGENT T** is the queen of trios. All significant operations flow through her 6-phase lotus cycle.

### 6-Phase Lotus Cycle

```
┌─────────────────────────────────────────────────────────────┐
│  PHASE 1: PLAN                                            │
│  • Analyze request (Swift UI, build fix, MCP, A2A)        │
│  • Read .trinity/experience.md for similar solved tasks   │
│  • Map to agent alphabet (A=arch, X=MCP, H=UI, etc.)    │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  PHASE 2: ASSIGN                                          │
│  • Pick agent(s) by domain match                          │
│  • Set dependency chain: A → C → F → V                    │
│  • Create tri-cell (W seals hash)                         │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  PHASE 3: RUN                                               │
│  • Spawn agent via Agent tool                               │
│  • Monitor via heartbeats (30s)                           │
│  • Log to .trinity/agent_events.jsonl                     │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  PHASE 4: TEST & BENCH                                    │
│  • F runs e2e (./build.sh + e2e/trios_e2e_flow.sh)        │
│  • M collects perf metrics                                  │
│  • V checks conformance against .trinity/baselines        │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  PHASE 5: VERDICT                                         │
│  • V analyzes: build pass? e2e pass? no regressions?      │
│  • If toxic → Q blocks, E records mistake                 │
│  • If clean → proceed to evolve                           │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  PHASE 6: EVOLVE                                          │
│  • E saves episode to .trinity/experience/                │
│  • W seals tri-cell commit (hash + timestamp)               │
│  • Z updates docs if API changed                            │
│  • T stamps TAW — work complete                           │
└─────────────────────────────────────────────────────────────┘
```

---

## ТРИ СЛОЯ АЛФАВИТА

### Нона I: Фундамент (A–I)
*Чистая концепция — Фундамент: душа, основа, типы*

Arch → Build → Comp → Migration → Experience → Conform → Graph → UI → ISA

### Нона II: Организм (J–R)
*Внутренний процесс — Жизнь системы: задачи, язык, числа, рантайм*

Jobs → Kernel → Lang → Metrics → Numeric → Orchestration → Physics → Queue → Runtime

### Нона III: Завершение (S–27th)
*Манифестация — Доказательство: стандарты, вердикт, деплой, дар*

Specs → Queen → Universe → Verdict → Workflow → eXternal → Yield → UX → Security

---

## Coordination by Letters (trios examples)

**Example: "Chat input broken, can't type or paste"**

1. **Plan** (T): Read `.trinity/experience.md` → found prior input fix using NSTextView
2. **Assign**: H (UI), K (macOS firstResponder), X (MCP bridge for testing)
3. **Run**: H redesigns input bar, K fixes WindowManager focus, X runs e2e
4. **Test**: F runs `./build.sh` + `bash e2e/trios_e2e_flow.sh`
5. **Verdict**: V confirms no regressions in other tabs
6. **Evolve**: E saves episode `input-nstextview-focus.json`

---

## Non-Negotiables for trios

1. **Specs are source of truth** — behavior belongs in `.claude/agents/` and `.claude/skills/`; generated Swift is not hand-edited without agent review.
2. **Build passes before land** — `./build.sh` must succeed; no merge with red build.
3. **English + ASCII** — first-party Markdown and source comments per L3 PURITY.
4. **No new .sh/.py on critical path** — L7 UNITY; use MCP tools or `build.sh` only.
5. **Issue gate** — PRs link issues (`Closes #N`) per L1 TRACEABILITY.
6. **Experience saved** — every non-trivial fix produces an episode in `.trinity/experience/`.

---

## φ² + 1/φ² = 3 | TRINITY
