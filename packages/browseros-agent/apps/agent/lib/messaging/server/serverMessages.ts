import { defineExtensionMessaging } from '@webext-core/messaging'
import type { McpTool } from '@/lib/mcp/client'

type ServerMessagesProtocol = {
  checkHealth(): { healthy: boolean }
  fetchMcpTools(): { tools: McpTool[]; error?: string }
}

/**
 * @public
 */
const { sendMessage, onMessage } =
  defineExtensionMessaging<ServerMessagesProtocol>()

export { onMessage as onServerMessage, sendMessage as sendServerMessage }
