import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { HarnessAgent, HarnessAgentAdapter } from './agent-harness-types'
import {
  type AgentListItem,
  CONTROL_PLANE_COPY,
  FALLBACK_CONTROL_PLANE_COPY,
  type GatewayUiState,
  LIFECYCLE_BANNER_COPY,
  type ProviderOption,
  RECOVERY_REASON_COPY,
} from './agents-page-types'
import { getOpenClawSupportedProviders } from './openclaw-supported-providers'
import {
  type AgentEntry,
  type GatewayLifecycleAction,
  getModelDisplayName,
  type OpenClawStatus,
} from './useOpenClaw'

export function getControlPlaneCopy(
  status: OpenClawStatus['controlPlaneStatus'],
) {
  return CONTROL_PLANE_COPY[status] ?? FALLBACK_CONTROL_PLANE_COPY
}

export function getRecoveryDetail(status: OpenClawStatus): string | null {
  if (!status.lastRecoveryReason && !status.lastGatewayError) return null

  const detail = status.lastRecoveryReason
    ? RECOVERY_REASON_COPY[status.lastRecoveryReason]
    : null

  if (status.lastGatewayError && detail) {
    return `${detail} Latest gateway error: ${status.lastGatewayError}`
  }

  return status.lastGatewayError ?? detail
}

export function formatHarnessAdapter(adapter: HarnessAgentAdapter): string {
  return adapter === 'claude' ? 'Claude Code' : 'Codex'
}

export function toProviderOptions(
  providers: LlmProviderConfig[],
  cliProviders: ProviderOption[],
): ProviderOption[] {
  return [...getOpenClawSupportedProviders(providers), ...cliProviders]
}

export function toOpenClawListItem(
  agent: AgentEntry,
  canManageAgents: boolean,
): AgentListItem {
  return {
    key: `openclaw:${agent.agentId}`,
    agentId: agent.agentId,
    name: agent.name,
    source: 'openclaw',
    runtimeLabel: 'OpenClaw',
    modelLabel: getModelDisplayName(agent.model) ?? 'default',
    detail: agent.workspace,
    canChat: canManageAgents,
    canDelete: canManageAgents && agent.agentId !== 'main',
  }
}

export function toHarnessListItem(agent: HarnessAgent): AgentListItem {
  return {
    key: `agent-harness:${agent.id}`,
    agentId: agent.id,
    name: agent.name,
    source: 'agent-harness',
    runtimeLabel: formatHarnessAdapter(agent.adapter),
    modelLabel: agent.modelId ?? 'default',
    detail: `${agent.adapter}:main`,
    canChat: true,
    canDelete: true,
  }
}

export function getGatewayUiState(
  status: OpenClawStatus | null,
): GatewayUiState {
  if (!status) {
    return {
      canManageAgents: false,
      controlPlaneDegraded: false,
      controlPlaneBusy: false,
    }
  }

  const controlPlaneBusy =
    status.controlPlaneStatus === 'connecting' ||
    status.controlPlaneStatus === 'reconnecting' ||
    status.controlPlaneStatus === 'recovering'

  return {
    canManageAgents:
      status.status === 'running' && status.controlPlaneStatus === 'connected',
    controlPlaneBusy,
    controlPlaneDegraded:
      status.status === 'running' && status.controlPlaneStatus !== 'connected',
  }
}

export function getLifecycleBanner(
  action: GatewayLifecycleAction | null,
): string | null {
  return action ? LIFECYCLE_BANNER_COPY[action] : null
}

export function canManageOpenClawAgents(
  state: GatewayUiState,
  lifecyclePending: boolean,
): boolean {
  return state.canManageAgents && !lifecyclePending
}

export function shouldShowControlPlaneDegraded(
  state: GatewayUiState,
  lifecyclePending: boolean,
): boolean {
  return state.controlPlaneDegraded && !lifecyclePending
}

export function getControlPlaneCopyForStatus(status: OpenClawStatus | null) {
  return status
    ? getControlPlaneCopy(status.controlPlaneStatus)
    : FALLBACK_CONTROL_PLANE_COPY
}

export function getVisibleOpenClawAgents(
  enabled: boolean,
  agents: AgentEntry[],
): AgentEntry[] {
  return enabled ? agents : []
}

export function getAgentsLoading(input: {
  statusLoading: boolean
  adaptersLoading: boolean
  harnessAgentsLoading: boolean
  openClawAgentsEnabled: boolean
  openClawAgentsLoading: boolean
}): boolean {
  return (
    input.statusLoading ||
    input.adaptersLoading ||
    input.harnessAgentsLoading ||
    (input.openClawAgentsEnabled && input.openClawAgentsLoading)
  )
}

export function getInlineError(input: {
  lifecyclePending: boolean
  pageError: string | null
  statusError: Error | null
  openClawAgentsError: Error | null
  adaptersError: Error | null
  harnessAgentsError: Error | null
}): string | null {
  if (input.lifecyclePending) return null
  return (
    input.pageError ??
    input.statusError?.message ??
    input.openClawAgentsError?.message ??
    input.adaptersError?.message ??
    input.harnessAgentsError?.message ??
    null
  )
}
