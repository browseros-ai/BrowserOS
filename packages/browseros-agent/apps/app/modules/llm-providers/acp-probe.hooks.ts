import { useQuery } from '@tanstack/react-query'
import type { ProviderType } from '@/lib/llm-providers/types'
import { useAgentServerUrl } from '../browseros/agent-server-url.hooks'

interface AcpProbeModel {
  id: string
  name?: string
  description?: string
}

interface AcpProbeReasoning {
  values: string[]
  defaultValue?: string
}

interface AcpProbeError {
  code: string
  message: string
}

interface AcpProbeResult {
  models: AcpProbeModel[]
  reasoning: AcpProbeReasoning | null
  supportsConfigOption: boolean
  agentInfo: { name?: string; title?: string; version?: string } | null
  protocolVersion: number
  error?: AcpProbeError
}

export interface UseAcpProbeOptions {
  providerType: ProviderType | undefined
  acpAgentId?: string
  command?: string
  cwd?: string
  enabled?: boolean
}

const BUILT_IN_AGENT_BY_TYPE: Partial<Record<ProviderType, string>> = {
  'claude-code': 'claude',
  codex: 'codex',
}

// Probe results encode the agent's currently-installed CLI version, which
// can change underfoot (npm install, codex-acp release). Refetch on every
// dialog open instead of trusting a stale memory cache.
const PROBE_STALE_TIME_MS = 0

export function resolveAcpAgentId(
  opts: UseAcpProbeOptions,
): string | undefined {
  if (opts.acpAgentId) return opts.acpAgentId
  if (!opts.providerType) return undefined
  return BUILT_IN_AGENT_BY_TYPE[opts.providerType]
}

export function isAcpProbeEnabled(
  opts: UseAcpProbeOptions,
  agentServerUrl: string | undefined,
  agentId: string | undefined,
): boolean {
  if (!(opts.enabled ?? true)) return false
  if (!agentServerUrl) return false
  if (!opts.providerType) return false
  if (opts.providerType === 'acp-custom') {
    return Boolean(opts.command) && Boolean(agentId)
  }
  return Boolean(agentId)
}

export function useAcpProbe(opts: UseAcpProbeOptions) {
  const { baseUrl: agentServerUrl } = useAgentServerUrl()
  const agentId = resolveAcpAgentId(opts)
  const enabled = isAcpProbeEnabled(opts, agentServerUrl ?? undefined, agentId)

  return useQuery<AcpProbeResult>({
    queryKey: [
      'acpx-probe',
      opts.providerType,
      agentId,
      opts.command,
      opts.cwd,
    ],
    enabled,
    staleTime: PROBE_STALE_TIME_MS,
    queryFn: async () => {
      const res = await fetch(`${agentServerUrl}/acpx/probe`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          agentId,
          command: opts.command,
          cwd: opts.cwd,
        }),
      })
      if (!res.ok) {
        const body = (await res.json().catch(() => ({}))) as {
          error?: AcpProbeError
        }
        throw new Error(body.error?.message ?? 'Probe request failed')
      }
      return (await res.json()) as AcpProbeResult
    },
  })
}
