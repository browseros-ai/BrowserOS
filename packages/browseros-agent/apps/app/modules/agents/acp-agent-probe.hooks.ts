import { ServerApiClient } from '@browseros/server-api/client'
import { useQuery } from '@tanstack/react-query'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import type { AcpAgentType } from './acp-agent-types'

export function useAcpAgentProbe(
  type: AcpAgentType | undefined,
  enabled = true,
) {
  const { baseUrl } = useAgentServerUrl()

  return useQuery({
    queryKey: ['acp-agent-probe', type, baseUrl],
    enabled: enabled && Boolean(type && baseUrl),
    staleTime: 0,
    queryFn: async () => {
      if (!type || !baseUrl)
        throw new Error('BrowserOS agent or server URL is not ready')
      return new ServerApiClient(baseUrl).probeAgent({ type })
    },
  })
}
