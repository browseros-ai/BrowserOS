import type { AgentProfile } from '@/modules/api/agents.hooks'

/**
 * Pulls the slug segment out of an MCP URL.
 * `http://127.0.0.1:9000/mcp/cowork-finance-ops` -> `cowork-finance-ops`.
 * Returns an empty string if the URL doesn't follow the `/mcp/<slug>` shape.
 */
export function slugFromMcpUrl(url: string): string {
  const match = url.match(/\/mcp\/([^/?#]+)/)
  return match?.[1] ?? ''
}

/**
 * Mirrors `new-agent.helpers.ts`'s buildCliCommand so the directory
 * shows the same CLI snippet the wizard's right rail printed when the
 * profile was created.
 */
export function cliCommandFor(profile: AgentProfile): string {
  const slug = slugFromMcpUrl(profile.mcpUrl) || profile.id
  return `mcp add ${slug}`
}
