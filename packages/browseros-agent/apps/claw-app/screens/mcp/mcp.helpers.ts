/**
 * TODO(v2-restore-per-agent): the v2 MCP page does not consume these
 * helpers because the v2 URL is slugless. Re-exports stay for the
 * legacy wizard and the new-agent screen, both of which still use
 * the slug builder when VITE_COCKPIT_LEGACY_UI=1.
 */

import type { AgentProfile } from '@/modules/api/agents.hooks'
import {
  buildMcpCliCommand,
  slugFromMcpEndpointUrl,
} from '@/modules/api/mcp-endpoint'

// Slug parsing is sourced from `@/modules/api/mcp-endpoint` so the
// directory and the wizard agree on the URL shape (the canonical
// builder lives there too). Re-exported so existing call sites do not
// have to change import paths.
export { slugFromMcpEndpointUrl as slugFromMcpUrl } from '@/modules/api/mcp-endpoint'

/**
 * Profile-shaped wrapper around `buildMcpCliCommand`. Falls back to
 * the profile id when the saved `mcpUrl` does not parse, which can
 * happen with legacy rows written before the cockpit-mount cutover.
 */
export function cliCommandFor(profile: AgentProfile): string {
  const slug = slugFromMcpEndpointUrl(profile.mcpUrl) || profile.id
  return buildMcpCliCommand(slug)
}
