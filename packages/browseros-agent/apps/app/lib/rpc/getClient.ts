import type { AppType } from '@browseros/server'
import { hc } from 'hono/client'
import { getAgentServerUrl } from '../browseros/helpers'

export type RpcClient = ReturnType<typeof hc<AppType>>

let clientPromise: Promise<RpcClient> | null = null

export const getClient = (): Promise<RpcClient> => {
  if (!clientPromise) {
    clientPromise = getAgentServerUrl().then((serverUrl) =>
      hc<AppType>(serverUrl),
    )
  }
  return clientPromise
}

// Pre-resolve the client immediately when the module is imported
getClient()
