import type { HarnessAgentAdapter } from './agent-harness-types'
import type { GatewayLifecycleAction, OpenClawStatus } from './useOpenClaw'

export type CreateAgentRuntime = 'openclaw' | HarnessAgentAdapter

export interface ProviderOption {
  id: string
  type: string
  name: string
  modelId: string
  baseUrl?: string
  apiKey?: string
}

export interface AgentListItem {
  key: string
  agentId: string
  name: string
  source: 'openclaw' | 'agent-harness'
  runtimeLabel: string
  modelLabel: string
  detail: string
  canChat: boolean
  canDelete: boolean
}

export interface GatewayUiState {
  canManageAgents: boolean
  controlPlaneDegraded: boolean
  controlPlaneBusy: boolean
}

export const DEFAULT_HARNESS_ADAPTER: HarnessAgentAdapter = 'claude'
export const DEFAULT_CREATE_RUNTIME: CreateAgentRuntime = 'openclaw'

export const LIFECYCLE_BANNER_COPY: Record<GatewayLifecycleAction, string> = {
  setup: 'Setting up OpenClaw...',
  start: 'Starting gateway...',
  stop: 'Stopping gateway...',
  restart: 'Restarting gateway...',
  reconnect: 'Restoring gateway connection...',
}

export const CONTROL_PLANE_COPY: Record<
  OpenClawStatus['controlPlaneStatus'],
  {
    badgeVariant: 'default' | 'secondary' | 'outline' | 'destructive'
    badgeLabel: string
    title: string
    description: string
  }
> = {
  connected: {
    badgeVariant: 'default',
    badgeLabel: 'Control Plane Ready',
    title: 'Gateway Connected',
    description: 'OpenClaw can create, manage, and chat with agents normally.',
  },
  connecting: {
    badgeVariant: 'secondary',
    badgeLabel: 'Connecting',
    title: 'Connecting to Gateway',
    description:
      'BrowserOS is establishing the OpenClaw control channel for agent operations.',
  },
  reconnecting: {
    badgeVariant: 'secondary',
    badgeLabel: 'Reconnecting',
    title: 'Reconnecting Control Plane',
    description:
      'The gateway process is up, but BrowserOS is restoring the control channel.',
  },
  recovering: {
    badgeVariant: 'secondary',
    badgeLabel: 'Recovering',
    title: 'Recovering Gateway Connection',
    description:
      'BrowserOS detected a control-plane fault and is trying a safe recovery path.',
  },
  disconnected: {
    badgeVariant: 'outline',
    badgeLabel: 'Disconnected',
    title: 'Gateway Disconnected',
    description: 'The gateway process is not available to BrowserOS right now.',
  },
  failed: {
    badgeVariant: 'destructive',
    badgeLabel: 'Needs Attention',
    title: 'Gateway Recovery Failed',
    description:
      'BrowserOS could not restore the OpenClaw control channel automatically.',
  },
}

export const FALLBACK_CONTROL_PLANE_COPY = {
  badgeVariant: 'outline' as const,
  badgeLabel: 'Unknown',
  title: 'Gateway State Unknown',
  description:
    'BrowserOS received a gateway status it does not recognize yet. Refreshing or reconnecting should restore a known state.',
}

export const RECOVERY_REASON_COPY: Record<
  NonNullable<OpenClawStatus['lastRecoveryReason']>,
  string
> = {
  transient_disconnect:
    'The control channel dropped briefly and BrowserOS is retrying it.',
  signature_expired:
    'The gateway rejected the signed device handshake because its clock drifted.',
  pairing_required:
    'The gateway asked BrowserOS to approve its local device identity again.',
  token_mismatch:
    'BrowserOS had to reload the gateway token before reconnecting.',
  container_not_ready:
    'The OpenClaw gateway process is not ready yet, so control-plane recovery cannot start.',
  unknown:
    'BrowserOS hit an unexpected gateway error and could not classify it cleanly.',
}
