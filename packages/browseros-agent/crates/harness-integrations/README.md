# harness-integrations

`harness-integrations` manages BrowserClaw integrations with AI coding harnesses. It has two runtime responsibilities:

- install and remove MCP server entries in each harness's real configuration file;
- reconcile product-owned agent skills into each harness's global skill directory.

Both domains consume one shared catalog of the seven supported harnesses: Claude Code, Codex, Cursor, OpenCode, Antigravity, VS Code, and Zed. Each catalog record owns the harness identity and install fingerprint together with nested MCP metadata and an optional skill surface, so path and capability facts cannot drift between independent catalogs.

## Module ownership

- `catalog.rs` contains shared harness identity, OS-specific definitions, and immutable MCP/skill surface metadata. It does not perform MCP writes or skill reconciliation.
- `mcp/` owns MCP request/result types, installation and config-path resolution, configuration emitters, manifest I/O, pure planning, and the workspace-bound `McpManager`.
- `skills/` owns skill targeting, ownership markers, the skill manifest, and `SkillReconciler`.
- `lib.rs` is the public facade. It keeps the established flat type and function exports even though their implementations live in domain modules.
- `error.rs` is the shared public error envelope returned by both domains.

### MCP state and writes

An MCP workspace `manifest.json` records which server entry the library wrote to each harness and the exact configuration path used. MCP changes follow three strict layers:

1. state I/O snapshots the manifest and requested harness files;
2. pure planners derive ordered filesystem operations and the next manifest without mutating the snapshot;
3. plan application executes atomic sibling-temp-file writes in order, then removals.

### Managed skill reconciliation

The skill reconciler resolves catalog-defined global roots against an explicit environment, groups harnesses that share a physical target, and converges product-owned skill content. A workspace `skills.json` manifest and a `.browserclaw-managed.json` marker establish ownership; foreign directories are preserved and reported as warnings.

Both APIs are synchronous. Async callers should use their runtime's blocking-task facility.

## Differences from the TypeScript package

- The shared catalog is limited to the seven BrowserClaw harness targets listed above.
- Emitters support JSON, JSONC, and TOML. YAML-only agents are outside this catalog.
- JSON and JSONC use `jsonc-parser`'s mutable CST so comments and untouched formatting survive edits.
- TOML uses `toml_edit`, preserving comments that the TypeScript package's TOML serializer loses.
- The TypeScript `remove` verb and `lowlevel` export are omitted because they have no production consumers. The read/plan/apply separation remains internal.
- Only system scope is implemented. `AgentScope::Project` is retained for API evolution and returns a clear error.
- Unlike the TypeScript package, `rescan` reads every manifest-recorded `configPath` instead of re-resolving OS defaults. This avoids false missing reports and duplicate healing writes after custom paths or environment variables change.
