# Provenance

Inlined snapshot of `acpx-ai-provider` from https://github.com/DaniAkash/acpx (monorepo path `packages/acpx-ai-provider`).

- Upstream commit: `ffb54400bf581e61c4cfcba3dccfcb72bc5cd44d`
- Upstream tag: `acpx-ai-provider-v0.0.6`
- Upstream version at snapshot: `0.0.6`
- AI SDK line: v6 (`ai >=6.0.0`, `@ai-sdk/provider ^3.0.10`, `@ai-sdk/provider-utils ^4.0.26`)
- Inlined on: 2026-07-21

## Why inlined

BrowserOS wants to edit the provider in place without a round-trip through npm publish. The upstream package continues to exist and may keep publishing separately; this copy is a hard fork with no automatic upstream sync.

## Version pin (v6, not v7)

Upstream `main` has since migrated to the AI SDK v7 beta line under the same `0.0.6` label (it demands `ai >=7.0.0-beta.0`, `@ai-sdk/provider@4-beta`, `@ai-sdk/provider-utils@5-beta`). BrowserOS runs `ai@6`, so this snapshot is deliberately taken at the `acpx-ai-provider-v0.0.6` tag, which is the last v6-compatible source and matches the published `acpx-ai-provider@0.0.6` dist that BrowserOS previously depended on. Do not refresh from `main` without also migrating BrowserOS to AI SDK v7.

## Divergence policy

Edits to this directory are made freely. There is no obligation to sync back to upstream. If upstream ships a v6-compatible bugfix worth pulling, apply it as an ordinary PR touching just this directory and update the upstream-commit SHA above so the divergence point stays discoverable.

## Third-party source

None incorporated. This is a clean-room provider built on the acpx runtime; its dependencies (`@ai-sdk/provider`, `@ai-sdk/provider-utils`, both MIT) are declared as ordinary package dependencies, not vendored source.

## Local patches (diverged from the snapshot above)

- Stripped explicit `.ts` extensions from internal relative imports across `src/` and `test/` (`from './x.ts'` becomes `from './x'`). Required because the consuming `apps/server` typechecks under `moduleResolution: bundler` without `allowImportingTsExtensions`, and it deep-checks the imported source; explicit `.ts` specifiers would trip TS5097. Matches the extensionless-import convention already used by the sibling `agent-mcp-manager` package.
- Verified clean against the repo's Biome config; `biome check` reports no changes (no reformatting was needed).
- Not vendored: `test/e2e/` (spawns real Claude/Codex/Gemini agents) and `bunup.config.ts` (no build step here; the package exports raw TS source).
