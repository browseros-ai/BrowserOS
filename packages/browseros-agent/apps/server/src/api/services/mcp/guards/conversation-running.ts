import { errorResult } from '@browseros/browser-mcp/tools/framework'
import type { BrowserToolGuard } from '../browser-tool-dispatch'

/** Internal leases authorize calls only while their conversation is running. */
export const guardConversationRunning: BrowserToolGuard = (call) => {
  if (!call.lease || call.run) return null
  return errorResult('MCP tools require an active conversation run.')
}
