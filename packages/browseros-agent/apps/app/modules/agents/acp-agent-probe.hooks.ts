import { useQuery } from '@tanstack/react-query'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import type { AcpAgentType, AcpProbeResult } from './acp-agent-types'

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
      const response = await fetch(`${baseUrl}/acpx/probe`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ type }),
      })
      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as {
          error?: { message?: string }
        }
        throw new Error(body.error?.message ?? 'Agent probe failed')
      }
      return response.json() as Promise<AcpProbeResult>
    },
  })
}
