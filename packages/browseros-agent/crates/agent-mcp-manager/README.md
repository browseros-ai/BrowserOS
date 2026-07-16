# agent-mcp-manager

`agent-mcp-manager` registers and deregisters MCP servers in the real configuration files used by AI coding agents. A workspace `manifest.json` records which server entry the library wrote to each agent and the exact configuration path used.

The implementation has three strict layers:

1. `read_state` snapshots the manifest and requested agent files.
2. Pure planners derive ordered filesystem operations and the next manifest without mutating the snapshot.
3. `apply_plan` executes atomic sibling-temp-file writes in plan order, then removals.

The public API is the workspace-bound `Manager` plus agent discovery and path-resolution helpers. Configuration writes are synchronous; async callers should use their runtime's blocking-task facility.

## Differences from the TypeScript package

- The catalog is limited to the seven BrowserClaw harness targets: Claude Code, Codex, Cursor, OpenCode, Antigravity, VS Code, and Zed.
- Emitters support JSON, JSONC, and TOML. YAML-only agents are outside this catalog.
- JSON and JSONC use `jsonc-parser`'s mutable CST so comments and untouched formatting survive edits.
- TOML uses `toml_edit`, preserving comments that the TypeScript package's TOML serializer loses.
- The TypeScript `remove` verb and `lowlevel` export are omitted because they have no production consumers. The read/plan/apply separation remains internal.
- Only system scope is implemented. `AgentScope::Project` is retained for API evolution and returns a clear error.
- Unlike the TypeScript package, `rescan` reads every manifest-recorded `configPath` instead of re-resolving OS defaults. This avoids false missing reports and duplicate healing writes after custom paths or environment variables change.
