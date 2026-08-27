import { useMutation, useQuery } from '@tanstack/react-query'
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
      const response = await fetch(`${baseUrl}/acpx/probe`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          type: 'custom',
          command,
          ...(env && Object.keys(env).length > 0 ? { env } : {}),
          ...(cwd ? { cwd } : {}),
        }),
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
