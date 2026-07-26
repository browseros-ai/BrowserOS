# Provenance

Inlined snapshot of `agent-mcp-manager` from https://github.com/DaniAkash/agent-toolkit.

- Upstream commit: `50ec1c8556015987c89f79d6855fec464b5aaea6`
- Upstream branch: `feat/mcp-manager-v0.0.4-fp`
- Upstream PR: https://github.com/DaniAkash/agent-toolkit/pull/64
- Upstream version at snapshot: `0.0.4-rc.4`
- Inlined on: 2026-07-10 (refresh from `fedf865d59...`)

## Why inlined

BrowserOS wants to edit this code in place without a round-trip through npm publish. The upstream package continues to exist and may keep publishing separately; this copy is a hard fork with no automatic upstream sync.

## Divergence policy

Edits to this directory are made freely. There is no obligation to sync back to upstream. If upstream ships a bugfix worth pulling, apply it as an ordinary PR touching just this directory and update the upstream-commit SHA above so the divergence point stays discoverable.

## Local patches (diverged from the snapshot above)

- `_catalog/client-configs.ts`: corrected the OpenCode paths. Current OpenCode (opencode.ai, 1.x) stores its global config at `$XDG_CONFIG_HOME/opencode/` or `~/.config/opencode/opencode.json`, not the legacy `~/.opencode/opencode.jsonc` the snapshot pointed at, so detection and config writes missed real installs. `installCheckPaths` / `systemPaths` now list the XDG and `~/.config` locations first and keep `~/.opencode` as a fallback. Worth upstreaming to agent-toolkit.
