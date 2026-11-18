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

`bun run test:neo-spec`, the server's browser tests (`apps/server/tests/__helpers__`), and the
claw-mcp contract suite (`bun run test:claw-mcp-contract`) keep their own launchers; run them
directly, not under `test:browser`. Use `bun run dev:claw:watch` for interactive app work.

For browser overrides or run artifacts, read [the live-test guide](CONTRIBUTING.md#run-live-browser-tests).

## Remote heavy jobs

Heavy jobs can run on a shared Linux pool instead of this machine. With a client profile from
the pool's administrator, run them from `packages/browseros-agent` in place of the local command:

```sh
bun run remote gate            # crates/neo-sdk/check.sh
bun run remote image           # build the SDK image (amd64), then run accept.sh on it
bun run remote spec --ci       # bun run test:neo-spec; suite names and flags pass through
bun run remote run -- <cmd>    # any other command
bun run remote queue           # jobs running and waiting
```

It uploads `packages/browseros-agent` as the working tree has it, uncommitted and untracked files
included; Git metadata, ignored files, `.env*`, key and credential files, and agent state stay
behind. Everything else goes verbatim: take out any token written inline in a file first, and
send only code you trust, since the machines are shared.

The command streams the job's log, exits with the job's own status, and keeps `run.log`,
`result.json` and artifacts in the directory it prints. A line starting `remote:` is the client
failing, not your change. When it means no client setup or an unreachable pool, run the step
locally and say so in your report: retrying does not help.

- Each worktree may hold two jobs, running plus waiting; a third is refused. Wait for one to end.
- A busy pool queues the job (gates first, then images, then the rest); let it wait. Stop a job
  whose result you no longer need: that frees its slot.
- Run `detector` and macOS-only work locally: the pool is Linux, and the detector does not yet
  complete there.

For client setup, `viewer-image`, `recover`, and what the machines retain, read
[the remote-builds guide](tools/remote-builds/README.md).

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
- New packages go in `packages/`, apps in `apps/`, Rust crates in `crates/`. A package exposes
  narrow `package.json` `exports` entries, each with `types` and `default`, rather than a
  package-wide `index.ts` barrel.

## Code comments

- **Default to explanatory comments for non-trivial code.** A new engineer should understand the
  architecture without reconstructing it across several files.
- Names explain local behavior. Comments explain system-level purpose, ownership, data flow, and
  important tradeoffs.
- Add comments at important seams: process or thread boundaries, async handoffs, lifecycle and
  cleanup, cancellation, permissions, protocol translation, request correlation, ordering, and
  persistence.
- Explain hidden constraints and what would break if an invariant were removed.
- New non-trivial modules and central types should have a concise docstring explaining their role
  in the larger system.
- Multi-step flows spanning several files may use a short block comment or ASCII diagram.
- Docstrings may use 1–3 concise sentences. Complex interfaces may use a short paragraph when
  necessary.
- Before finishing a non-trivial change, review every modified file for missing architectural
  context.
- Keep straightforward code uncluttered. Avoid line-by-line paraphrases, PR narration, stale TODOs,
  and commented-out code.

## Where to look

- Server work: `apps/server/CLAUDE.md`.
- Extension and app UI work: `apps/app/CLAUDE.md`.
- Neo SDK crates: `crates/neo-sdk/STYLE.md` holds the rules review checks, and
  `crates/neo-sdk/check.sh` runs them.
