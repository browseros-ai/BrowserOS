# BrowserOS Agent contributor ground rules

A Bun-workspaces monorepo for the BrowserOS MCP server, agent extension UI, and shared packages.

## Before you push

Run the root check suite and the tests:

```
bun run check
bun run test
```

`bun run check` runs lint, typecheck, and Fallow. `bun run test` runs the full test suite. For docs-only changes, also run `git diff --check`. For release/build changes, run the relevant `bun run build:*` command.

## Live browser tests

For standalone live tests that expect `BROWSEROS_CDP_PORT`, run from `packages/browseros-agent`:

```sh
bun run test:browser -- <existing test command>
```

Use one invocation per independent run; it supplies the port and owns the private Neo and
cleanup. Preserve your test command and flags.

Keep `test:neo-spec` and full MCP fixtures on their existing launchers. Use `dev:claw:watch`
for interactive app work.

For browser overrides or cleanup failures, read [the live-test guide](CONTRIBUTING.md#run-live-browser-tests).

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
- For extension/app UI specifics, see `apps/app/CLAUDE.md`.
