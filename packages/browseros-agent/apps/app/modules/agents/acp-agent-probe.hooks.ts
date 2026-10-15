import { useMutation, useQuery } from '@tanstack/react-query'
import { serverApi, serverData } from '@/lib/server-api/client'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import type { AcpAgentType, AcpProbeResult } from './acp-agent-types'

export interface ProbeCustomAgentInput {
  command: string
  env?: Record<string, string>
  cwd?: string
}

/**
 * Probe a not-yet-saved custom agent by its command. Manual (mutation) rather
 * than a query so the settings "Test connection" button drives it.
 */
export function useProbeCustomAgent() {
  const { baseUrl } = useAgentServerUrl()

  return useMutation<AcpProbeResult, Error, ProbeCustomAgentInput>({
    mutationFn: async ({ command, env, cwd }) => {
      if (!baseUrl) throw new Error('BrowserOS agent server URL is not ready')
      return serverData(
        serverApi(baseUrl).http.POST('/acpx/probe', {
          body: {
            type: 'custom',
            command,
            ...(env && Object.keys(env).length > 0 ? { env } : {}),
            ...(cwd ? { cwd } : {}),
          },
        }),
      )
    },
  })
}

export function useAcpAgentProbe(
  type: AcpAgentType | undefined,
  enabled = true,
) {
  const { baseUrl } = useAgentServerUrl()

  return useQuery<AcpProbeResult>({
    queryKey: ['acp-agent-probe', type, baseUrl],
    enabled: enabled && Boolean(type && baseUrl),
    staleTime: 0,
    queryFn: async () => {
      if (!baseUrl || !type || type === 'custom')
        throw new Error('Agent probe failed')
      return serverData(
        serverApi(baseUrl).http.POST('/acpx/probe', { body: { type } }),
      )
    },
  })
}
