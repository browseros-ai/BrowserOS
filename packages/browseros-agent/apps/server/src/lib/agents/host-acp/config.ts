/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export type HostAcpAdapter = 'claude' | 'codex'

interface HostAcpAdapterConfig {
  acpArgv: readonly string[]
  acpPackageSpec: string
  acpBin: string
}

/**
 * Runtime npm packages launched by both chat and provider discovery. Their version
 * ranges also key Bun's executable cache, so raise the minimum when a release needs
 * newer agent support (Codex ACP 1.10.0 brings the CLI required by Astra).
 * These adapters carry their own agent runtimes; updating a host CLI is insufficient.
 */
export const HOST_ACP_ADAPTER_CONFIG = {
  claude: {
    acpArgv: ['npx', '-y', '@agentclientprotocol/claude-agent-acp@^0.75.1'],
    acpPackageSpec: '@agentclientprotocol/claude-agent-acp@^0.75.1',
    acpBin: 'claude-agent-acp',
  },
  codex: {
    acpArgv: ['npx', '-y', '@agentclientprotocol/codex-acp@^1.10.0'],
    acpPackageSpec: '@agentclientprotocol/codex-acp@^1.10.0',
    acpBin: 'codex-acp',
  },
} as const satisfies Record<HostAcpAdapter, HostAcpAdapterConfig>

/** Full-access mode ids in preference order; Codex keeps the legacy Zed alias as fallback. */
export const DANGEROUS_ALLOW_MODE_CANDIDATES: Readonly<
  Record<HostAcpAdapter, readonly string[]>
> = {
  claude: ['bypassPermissions'],
  codex: ['agent-full-access', 'full-access'],
}
