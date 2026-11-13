# BrowserOS Agent contributor ground rules

A monorepo for the BrowserOS MCP server, agent extension UI, Neo SDK, and shared packages: Bun
workspaces for TypeScript (`apps/`, `packages/`) and a Cargo workspace for Rust (`crates/`,
`apps/claw-server`).

## Before you push

```sh
bun run check   # lint, typecheck, Fallow
bun run test    # the Bun test suites; Rust is separate
```

After Rust changes, also run `bun run test:rust`, `bun run lint:rust`, and `bun run fmt:rust`.
After docs-only changes, also run `git diff --check`. After release or build changes, run the
matching `bun run build:*` command.

## Live browser tests

Run a test that reads `BROWSEROS_CDP_PORT` through the launcher, keeping its own command and
flags:

```sh
bun run test:browser -- cargo test -p neo-playwright --test pages_live -- --ignored
```

Each run starts a fresh headless Neo on a free port, supplies the port, and cleans up; start no
browser and set no port beforehand. Give every independent test command its own run, because
suites that share a browser clear each other's tabs and cookies. Never point a test at the
installed app: its bundled server reconciles your real MCP configuration and skills. Logs and
`result.json` stay in the artifact directory printed to stderr. `NEO_E2E_BROWSER` selects
another browser build. The launcher needs Node 22+.

`bun run test:neo-spec` and the full MCP fixtures launch their own browsers; run them directly,
not under `test:browser`. Interactive app work uses `bun run dev:claw:watch`.

## Conventions

- Use Bun for TypeScript: `bun <file>`, `bun test`, `bun install`, `bun run <script>`. Bun loads
  env files itself, so add dotenv only when a script proves it needs it.
- Write extensionless TypeScript imports: `./utils`, not `./utils.js`.
- Name folders and multi-word non-component files in kebab-case; `apps/app/CLAUDE.md` lists the
  UI exceptions.
- Put shared constants and types in `@browseros/shared` (entry points in its `package.json`
  `exports`) instead of inline magic values.
- Write logger messages as plain text. Development logging already prefixes file, line, and
  function, so a `[prefix]` tag repeats it.
- Keep comments minimal. Comment hidden constraints, subtle invariants, critical warnings, or
  surprising behavior; do not restate obvious code.
- New packages go in `packages/`, apps in `apps/`, Rust crates in `crates/`. A package exposes
  narrow `package.json` `exports` entries, each with `types` and `default`, rather than a
  package-wide `index.ts` barrel.

## Where to look

- Server work: `apps/server/CLAUDE.md`.
- Extension and app UI work: `apps/app/CLAUDE.md`.
- Neo SDK crates: `crates/neo-sdk/STYLE.md` holds the rules review checks, and
  `crates/neo-sdk/check.sh` runs them.
