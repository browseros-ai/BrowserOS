# Provenance

Inlined snapshot of `acpx-ai-provider` from https://github.com/DaniAkash/agent-toolkit (monorepo path `packages/acpx-ai-provider`). The repository was previously named `DaniAkash/acpx`; historical metadata in the pinned tag may still use that name.

- Upstream commit: `ffb54400bf581e61c4cfcba3dccfcb72bc5cd44d`
- Upstream tag: `acpx-ai-provider-v0.0.6`
- Upstream version at snapshot: `0.0.6`
- AI SDK line: v6 (`ai >=6.0.0`, `@ai-sdk/provider ^3.0.10`, `@ai-sdk/provider-utils ^4.0.26`)
- Inlined on: 2026-07-21

## Why inlined

BrowserOS wants to edit the provider in place without a round-trip through npm publish. The upstream package continues to exist and may keep publishing separately; this copy is a hard fork with no automatic upstream sync.

## Current compatibility and upstream review

The original snapshot targeted AI SDK v6. BrowserOS has since migrated this fork to AI SDK v7 (`@ai-sdk/provider ^4`, `@ai-sdk/provider-utils ^5`) and incorporated upstream changes. The workspace package is now version `1.0.0`; the snapshot version above records its original provenance.

Reviewed against upstream `main` at `1afa195fd576ff4d8af7859c5af8e2d672639ef5` on 2026-09-08; the latest published provider was `1.0.0`. Live usage events, available-command subscriptions, slash-command event routing, defensive-copy getters, tool-input ID correlation, and the explicit event-emitter type were already present locally. Ported the remaining selected-agent compaction lookup fix from upstream commit `0390bb75183bce4a51a13932475c20ea52aada53` (#80).

Upstream's event translator still groups only by text/reasoning stream at that revision. The message-boundary handling described below is a BrowserOS fix.

Also synchronized five stale usage-test expectations with upstream: `size` is exposed as `providerMetadata.acpx.contextWindow`, not as cached input tokens. The implementation already followed this contract.

## Divergence policy

Edits to this directory are made freely. There is no obligation to sync back to upstream. Apply compatible upstream fixes as ordinary changes to this directory and record their revisions here. Preserve the original snapshot revision above and review local adaptations before replacing source wholesale.

## Third-party source

None incorporated. This is a clean-room provider built on the acpx runtime; its dependencies (`@ai-sdk/provider`, `@ai-sdk/provider-utils`, both MIT) are declared as ordinary package dependencies, not vendored source.

## Local patches (diverged from the snapshot above)

- Stripped explicit `.ts` extensions from internal relative imports across `src/` and `test/` (`from './x.ts'` becomes `from './x'`). Required because the consuming `apps/server` typechecks under `moduleResolution: bundler` without `allowImportingTsExtensions`, and it deep-checks the imported source; explicit `.ts` specifiers would trip TS5097. Matches the extensionless-import convention already used by the sibling `agent-mcp-manager` package.
- Verified clean against the repo's Biome config; `biome check` reports no changes (no reformatting was needed).
- Not vendored: `test/e2e/` (spawns real Claude/Codex/Gemini agents) and `bunup.config.ts` (no build step here; the package exports raw TS source).
- AI SDK v7 file-data normalization and inline text/JSON attachments; these are not present in the upstream source reviewed above.
- Optional persistent-state deletion on `close()` and structured argv in agent-registry overrides.
- ACP `messageId` boundaries produce separate AI SDK text/reasoning blocks. Missing IDs continue the current block; a newly identified message separates preceding untagged text. Text is preserved verbatim, and generated block IDs avoid collisions across tools and text/reasoning transitions.
