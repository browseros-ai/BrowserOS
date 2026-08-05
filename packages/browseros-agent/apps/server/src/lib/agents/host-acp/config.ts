/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export type HostAcpAdapter = 'claude' | 'codex'

interface HostAcpAdapterConfig {
  acpCommand: string
  acpPackageSpec: string
  acpBin: string
}

export const HOST_ACP_ADAPTER_CONFIG = {
  claude: {
    acpCommand: 'npx -y @agentclientprotocol/claude-agent-acp@^0.31.0',
    acpPackageSpec: '@agentclientprotocol/claude-agent-acp@^0.31.0',
    acpBin: 'claude-agent-acp',
  },
  codex: {
    acpCommand: 'npx -y @agentclientprotocol/codex-acp@^1.0.2',
    acpPackageSpec: '@agentclientprotocol/codex-acp@^1.0.2',
    acpBin: 'codex-acp',
  },
} as const satisfies Record<HostAcpAdapter, HostAcpAdapterConfig>

/**
 * Full-permission ACP session modes per built-in adapter — the ACP
 * equivalent of `claude --dangerously-skip-permissions` / `codex
 * --dangerously-bypass-approvals-and-sandbox`. Without this override the
 * adapter inherits the user's own CLI defaults (e.g. Claude settings
 * `permissions.defaultMode: "dontAsk"`, which auto-denies the BrowserOS
 * MCP tools). Candidates are tried in order; codex lists two ids because
 * @agentclientprotocol/codex-acp advertises `agent-full-access`; older
 * Zed-packaged Codex ACP builds used `full-access` for the same
 * approval=never + danger-full-access preset, so keep it as a fallback.
 */
export const DANGEROUS_ALLOW_MODE_CANDIDATES: Readonly<
  Record<HostAcpAdapter, readonly string[]>
> = {
  claude: ['bypassPermissions'],
  codex: ['agent-full-access', 'full-access'],
}
