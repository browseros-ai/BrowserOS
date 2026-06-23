# BrowserOS Agent contributor ground rules

A Bun-workspaces monorepo for the BrowserOS MCP server, agent extension UI, CLI, eval harness, and shared packages.

## Before you push

Run the root check suite and the tests:

```
bun run check
bun run test
```

`bun run check` runs lint, typecheck, and Fallow. `bun run test` runs the full test suite. For docs-only changes, also run `git diff --check`. For release/build changes, run the relevant `bun run build:*` command.

## Universal rules

- Use extensionless TypeScript imports: `./utils`, not `./utils.js`.
- Use Bun: `bun <file>`, `bun test`, `bun install`, `bun run <script>`. Bun loads env files; do not add dotenv unless a script proves it needs one.
- Use kebab-case for folders and multi-word non-component files; app-specific framework exceptions live in the app CLAUDE files.
- Keep shared constants in `@browseros/shared` instead of scattering magic values:
  - `@browseros/shared/constants/ports`
  - `@browseros/shared/constants/timeouts`
  - `@browseros/shared/constants/limits`
  - `@browseros/shared/constants/urls`
  - `@browseros/shared/constants/paths`
  - `@browseros/shared/types/logger`
- Logger messages should not include `[prefix]` tags; development logging already adds file, line, and function.
- Keep comments minimal. Comment hidden constraints, subtle invariants, critical warnings, or surprising behavior; do not restate obvious code.
- New packages go in `packages/`; apps go in `apps/`. Avoid package-wide `index.ts` barrels and expose narrow files with both `types` and `default` entries in `package.json`.

## Where to look

- For server-specific guidance, see `apps/server/CLAUDE.md`.
- For extension/agent UI specifics, see `apps/agent/CLAUDE.md`.
- For the BrowserOS CLI (Go module — Go idioms, not the TS rules above), see `apps/cli/CLAUDE.md`.
- For the eval harness (benchmarks, graders, suites, runs), see `apps/eval/CLAUDE.md`.
