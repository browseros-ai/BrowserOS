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

For standalone tests that read `BROWSEROS_CDP_PORT`, run from `packages/browseros-agent`,
keeping the test's own command and flags:

```sh
bun run test:browser -- cargo test -p neo-playwright --test pages_live -- --ignored
```

Each run starts a private headless Neo, supplies a free port, and cleans up. Give independent
test commands their own runs: suites can clear shared tabs and cookies. The launcher disables
the bundled server and isolates the browser home to protect real MCP configuration and
shared skills. It requires Node 22+.

`bun run test:neo-spec` and full MCP fixtures keep their own launchers. Use
`bun run dev:claw:watch` for interactive app work.

For browser overrides or run artifacts, read [the live-test guide](CONTRIBUTING.md#run-live-browser-tests).

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
