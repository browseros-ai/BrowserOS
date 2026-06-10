# BrowserOS Eval contributor ground rules

The eval harness (`@browseros/eval`) runs browser-automation benchmarks against BrowserOS: load a task dataset, drive an agent through the BrowserOS MCP/CDP loop, capture trajectories + screenshots, grade the results, and optionally publish a run to R2. `README.md` is the user-facing run guide — this file is the contributor map.

## Before you push

From the monorepo root:

```
bun run lint
bun run typecheck
bun run --filter @browseros/eval test
```

`bun run test:main` does NOT exercise eval — it is server-only (`scripts/run-test-suite.ts`); eval tests live in the `all` suite. Use the `--filter` form above (or `cd apps/eval && bun run test`) or your change goes untested.

Tests run through the shared runner with **cwd = the monorepo root** (`packages/browseros-agent`), so fixture paths inside tests are workspace-root-relative (`apps/eval/configs/...`) and test-relative paths use `import.meta.dir`. Formatting is Biome (`bun run lint`); don't hand-fight its no-semicolon / single-quote style.

## Project shape

```
apps/eval/
|- src/
|  |- cli/          Command dispatch: suite | run | grade | publish | legacy (-c)
|  |- runs/         ACTIVE run pipeline: eval-runner, task-worker-pool, task-run-pipeline
|  |- runner/       Older substrate, still live: task-loader + shared run types
|  |- suites/       EvalSuite schema, suite loader, variant (model) resolution
|  |- agents/       Agent evaluators: single, orchestrator-executor, claude-code
|  |- graders/      Grader CLASSES (benchmark/, performance/) + python/ scripts
|  |- grading/      Grader orchestration: registry (factory), runner, python bridge
|  |- types/        Zod schemas + inferred types — the source of truth
|  |- utils/        Env / provider resolution, MCP client, helpers
|  |- capture/      Per-task message / screenshot / trajectory capture
|  |- reporting/    Run + task summaries
|  |- publishing/   R2 upload + run manifest
|  |- viewer/       Public viewer manifest contract
|  |- dashboard/    Hono live dashboard (http://localhost:9900)
|  `- constants.ts  Eval-only magic values
|- configs/
|  |- suites/       EvalSuite JSON; model comes from variant (CLI/env)
|  `- legacy/       Full EvalConfig JSON (dashboard + the -c form)
|- data/            Committed *.jsonl task sets (data/raw/ and results/ are gitignored)
|- scripts/         Dataset builders + reporting (mix of .ts and .py)
`- tests/           bun:test, mirrors src/
```

## How a run is wired

`src/index.ts` → `runCli` (`src/cli/index.ts`) dispatches on the first arg. The active runner is `src/runs/eval-runner.ts`; `suite`, `run`, and the legacy `-c`/dashboard path all route to it.

**Suite vs config.** A `configs/legacy/*.json` is a complete `EvalConfig`. A `configs/suites/*.json` is an `EvalSuite` (dataset + graders + browser settings) whose model comes from a *variant* — CLI flags first, then `EVAL_AGENT_*` env. `resolveSuiteCommand` / `suiteToEvalConfig` (`src/cli/commands/suite.ts`) adapt a suite into an `EvalConfig` before the runner sees it, and that adapter maps the suite agent vocabulary onto the runtime one (`tool-loop`→`single`, `orchestrated`→`orchestrator-executor`). The suite schema (`src/suites/schema.ts`) deliberately accepts more agent `type`s than the runtime factory implements.

## Adding a grader

1. Implement the `Grader` interface (`src/grading/types.ts`: `name` + `grade(input): Promise<GraderResult>`) under `src/graders/` — `benchmark/` for deterministic, `performance/` for the LLM-judge.
2. Register it in `src/grading/grader-registry.ts`: add a `createGrader` case, and if it yields a pass/fail add it to `PASS_FAIL_GRADER_ORDER` (the fallback priority for a task's headline grader).
3. `src/graders/registry.ts` is a back-compat shim that re-exports `src/grading` — edit `src/grading`, never the shim.

Python evaluator scripts (`agisdk-evaluate.py`, `infinity-evaluate.py`) must stay in `src/graders/python/`; `tests/grading/python-script-layout.test.ts` fails if they move to `scripts/`. They are spawned via `BROWSEROS_EVAL_PYTHON` through `src/grading/python-evaluator.ts`.

## Adding an agent type

Implement `AgentEvaluator` (`src/agents/types.ts`: `execute(): Promise<AgentResult>`), add a `createAgent` case (`src/agents/index.ts`), and extend the `AgentConfigSchema` discriminated union (`src/types/config.ts`). To make it suite-runnable, also extend `SuiteAgentSchema` and the `suiteToEvalConfig` adapter.

## Conventions & gotchas

- **Types are Zod-first.** Everything in `src/types/*` is an `XSchema` + `type X = z.infer<typeof XSchema>`, consumed via `from '../types'`. `EvalConfigSchema` is the validation gate (`.parse()` on every config/suite). Provider/model shapes extend `@browseros/shared/schemas/llm` — don't redefine them here.
- **Never leak API keys into persisted output.** `resolveVariant` (`src/suites/resolve-variant.ts`) returns a raw `agent` (with the key) and a `publicMetadata` that drops it (exposing `apiKeyConfigured` / `apiKeyEnv` / `baseUrlHost`). Manifests, reports, and the dashboard get `publicMetadata`; `tests/suites/schema.test.ts` asserts no raw secret ever appears.
- **Config keys are env-var NAMES, not secrets.** An ALL_CAPS `apiKey` value is resolved from `process.env` at runtime (`src/utils/resolve-env.ts`); anything else is used literally. Keep `.env.example` current when you add a variable.
- **The viewer manifest is a contract.** Built by `src/viewer/viewer-manifest.ts` (versioned, with per-task `paths`); published to R2 as `runs/<id>/manifest.json` (`src/publishing/r2-publisher.ts`) — don't confuse it with the run's `run.json`. Keep it and its tests green when you change artifact layout.
- **Eval-only constants** live in `src/constants.ts`; cross-package values come from `@browseros/shared/constants/*`.
- The `@eval/*` tsconfig path alias is defined but unused — follow the relative, extensionless imports used throughout `src/`.
