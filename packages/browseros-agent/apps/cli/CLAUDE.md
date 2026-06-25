# BrowserOS CLI contributor ground rules

`browseros-cli` is a Go (1.25+) Cobra CLI that drives BrowserOS by calling the `apps/server` MCP server over StreamableHTTP. Each command maps to one or more server MCP tools.

> This is the one Go module among the monorepo's TS apps. Go idioms win here: `gofmt`, lowercase/`snake_case` filenames (e.g. `file_actions.go`), `%w` error wrapping. The parent doc's extensionless-import, Bun, and kebab-case rules do **not** apply.

## Before you push

Built and tested with Go, not Bun — the monorepo-root `bun run lint/typecheck/test` does not cover this package. Run the Go checks from `apps/cli/`:

```
gofmt -l .          # must print nothing
go vet ./...        # or: make vet
go build ./...
go test ./...       # unit tests, no server needed
```

There is no `make lint`. Integration tests are separate (see Testing).

## Package map

```
apps/cli/
|- main.go            version var (ldflags) -> cmd.Execute
|- cmd/               one Cobra command per file, wired via init()
|  |- root.go         root cmd, global flags, grouped help, newClient, resolvePageID, URL handling, auto-update
|  `- *.go            command implementations (open.go, click.go, window.go, ...)
|- mcp/               MCP client: stateless connect -> tools/call -> close; ToolResult
|- output/            human + JSON formatting; Error/Errorf exit helpers
|- config/            ~/.config/browseros-cli/config.yaml (server_url)
|- analytics/         PostHog, fire-and-forget; no-op without an injected key
|- update/            self-update + background "update available" check
|- npm/               JS shim package that downloads the Go binary
|- scripts/           install.sh / install.ps1 (CDN installers)
`- Makefile           build / vet / test / release / npm-publish
```

## Adding a command

Pattern: see `cmd/open.go` and `cmd/snap.go`; grouped commands in `cmd/window.go`.

- One file per command (or command family) in `cmd/`. Register it with a package-level `func init()` that builds a `*cobra.Command` and calls `rootCmd.AddCommand(...)`. There is **no central registry** — `init()` side effects do the wiring, so a new file is enough.
- Set `Annotations: map[string]string{"group": "Navigate:"}`. The group must be one of `Navigate:` `Observe:` `Input:` `Resources:` `Integrations:` `Setup:` (`groupOrder` in `cmd/root.go`) — the trailing colon is required, and an unknown or empty group silently lands under `Setup:`.
- Grouped commands (`window`, `bookmark`, `history`, `group`): a parent command with no `Run`, children added via `parent.AddCommand(...)`; annotate only the parent.
- Body shape: validate args → `c := newClient()` → (if page-scoped) `pageID, err := resolvePageID(c)` → `c.CallTool("<tool>", map[string]any{...})` → branch on `jsonOut` for output. `newClient()` already exits with setup instructions on a missing/invalid server URL, so commands don't handle that case.

### Conventions inside a command

- **MCP tool names are the contract.** A command only shapes args; the server owns the tool. Keep command tool names and arg keys in sync with the compact MCP surface (`snap` → `snapshot`, `fill` → `act` with `kind=fill`).
- **Page targeting:** always resolve through `resolvePageID(c)` (`--page/-p` flag > `BROWSEROS_PAGE` env > server `get_active_page`) and pass it as the `"page"` arg. Don't reinvent it (`cmd/click.go`).
- **Output:** branch on the global `jsonOut`. JSON path → `output.JSON(result)` (emits `structuredContent` when present). Human path → `output.Text` / `output.Confirm` / a domain formatter (`output.PageList`, `output.ActivePage`). Color comes from `fatih/color` and auto-disables off a TTY.
- **Errors & exit codes:** route every error through `output.Error(msg, code)` / `output.Errorf(code, ...)` — red, to stderr, and they `os.Exit`. Never `fmt.Println` an error or call `os.Exit` directly. Codes follow a convention across `cmd/`: **1** = tool/RPC call failed, **2** = page resolution failed (`health`/`status` reuse it for an unreachable server), **3** = invalid CLI argument.

## MCP client (`mcp/`)

Stateless per command: `CallTool` opens a fresh StreamableHTTP session (handshake → `tools/call` → close) via `github.com/modelcontextprotocol/go-sdk`. No pooling, no long-lived session. `ToolResult` (`mcp/types.go`) exposes `.TextContent()`, `.ImageContent()`, and `.StructuredContent` (`map[string]any`). `Health()` / `Status()` are plain REST GETs (`/health`, `/status`), **not** MCP.

**Server-URL gotcha:** `normalizeServerURL` (`cmd/root.go`) strips a trailing `/mcp` and `/`, and expands a bare port (`9000` → `http://127.0.0.1:9000`). The stored base has **no** `/mcp`; the transport re-appends it in `mcp/client.go`. Store and compare the base without `/mcp`; never hand-append it. Resolution priority: `--server` flag > `BROWSEROS_URL` env > config file. BrowserOS writes a runtime discovery file, but commands intentionally ignore it (see the `defaultServerURL` doc comment) so a saved URL isn't silently overridden by another running server.

## Build, version & analytics injection

`make` builds with `-ldflags -X` injecting two private vars (`Makefile`):

- `main.version` — defaults to `"dev"` under a plain `go build`. Self-update refuses to run on non-release (`dev`) versions (`update.IsReleaseVersion`).
- `browseros-cli/analytics.posthogAPIKey` — empty in local/dev builds, so `analytics.Init` returns early and tracking is a **no-op**. It's injected only for production via `POSTHOG_API_KEY` (`.env.production.example`).

Never hard-code the version or the key — they are build-time ldflags only. Analytics (`analytics/`) is fire-and-forget: `Init`/`Track`/`Close` run once in `cmd.Execute()`; commands never call it directly. The distinct id is the BrowserOS id (`~/.browseros/server.json`) or a generated per-install UUID under the config dir; no PII is sent.

## Auto-update (`update/`)

`Execute()` fires a background check (~daily, 24h TTL) and prints a cached "update available" notice on a *later* run. It is skipped for the `help`, `completion`, and `update`/`self-update`/`upgrade` commands, for `--version`/`-h`, when `--json` is set, when `BROWSEROS_SKIP_UPDATE_CHECK` is set, for non-release builds, and when installed via a package manager (`BROWSEROS_INSTALL_METHOD=npm|brew`). `update` downloads from `cdn.browseros.com/cli/latest/manifest.json`, verifies the SHA-256, then atomically replaces the binary (`minio/selfupdate`).

## Config (`config/`)

One YAML file at `~/.config/browseros-cli/config.yaml` (`$XDG_CONFIG_HOME` honored). The only field is `server_url`. `config.Load()` returns an empty `&Config{}` — not an error — when the file is missing. Keep the struct minimal.

## Testing

- Unit tests are plain `go test ./...` and need no server (`cmd/root_test.go`, `update/*_test.go`, `mcp/client_test.go`, `analytics/analytics_test.go`).
- `integration_test.go` is behind `//go:build integration` and drives the **built binary** against a running dev server. `TestMain` health-probes `BROWSEROS_URL` (default `:9105`) and **skips gracefully** (exit 0) when none is reachable. Run with `make test` (`go test -tags integration`). Assertions are on stdout / stderr / exit code, usually via `--json`.

Add unit tests for pure logic (URL/version/arg parsing); add an integration test when behavior is only observable end-to-end.

## Release & npm distribution

- `make release VERSION=x.y.z` cross-compiles 6 targets (darwin/linux/windows × amd64/arm64), strips symbols, tar/zips them, writes `checksums.txt`, and **fails** unless the freshly built host binary reports `VERSION`. Artifacts land in `dist/`.
- Artifact names are a contract: `browseros-cli_<version>_<os>_<arch>.<tar.gz|zip>`. The npm postinstall and `checksums.txt` both depend on that exact name — don't rename casually.
- npm (`npm/`) ships a **thin JS shim, not the Go binary**: `bin/browseros-cli.js` execs the platform binary; `scripts/postinstall.js` downloads and checksum-verifies it from the matching GitHub Release into `npm/.binary/`. The npm version and the Go release tag **must match** — the postinstall URL is built from `package.json`'s version. Bump with `make npm-version VERSION=...`, publish with `make npm-publish`. Postinstall is skipped in CI unless `BROWSEROS_NPM_FORCE=1` (the binary then lazy-downloads on first run); the shim sets `BROWSEROS_INSTALL_METHOD=npm` so the binary suppresses self-update.

The server side of this contract lives in `apps/server/CLAUDE.md`.
