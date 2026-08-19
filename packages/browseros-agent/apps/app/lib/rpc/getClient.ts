import type { AgentRoutes, ConversationRoutes } from '@browseros/server'
import { hc } from 'hono/client'
import { getAgentServerUrl } from '../browseros/helpers'

// Per-domain typed clients. The server exports one contract per protected route
// group (see rpc.ts); each client is pointed at that group's mount path so the
// paths address directly (client.conversations.index.$get()).
export interface RpcClient {
  conversations: ReturnType<typeof hc<ConversationRoutes>>
  agents: ReturnType<typeof hc<AgentRoutes>>
}

let clientPromise: Promise<RpcClient> | null = null

export const getClient = (): Promise<RpcClient> => {
  if (!clientPromise) {
    clientPromise = getAgentServerUrl().then((serverUrl) => ({
      conversations: hc<ConversationRoutes>(`${serverUrl}/conversations`),
      agents: hc<AgentRoutes>(`${serverUrl}/agents`),
    }))
  }
  return clientPromise
}
