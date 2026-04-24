import {
  AlertCircle,
  Cpu,
  Loader2,
  MessageSquare,
  Plus,
  RefreshCw,
  ShieldAlert,
  Square,
  TerminalSquare,
  Trash2,
  WifiOff,
  Wrench,
} from 'lucide-react'
import { type FC, useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { useLlmProviders } from '@/lib/llm-providers/useLlmProviders'
import { AgentTerminal } from './AgentTerminal'
import { getOpenClawSupportedProviders } from './openclaw-supported-providers'
import {
  type AgentEntry,
  type GatewayLifecycleAction,
  type OpenClawStatus,
  useOpenClawAgents,
  useOpenClawMutations,
  useOpenClawStatus,
} from './useOpenClaw'

const LIFECYCLE_BANNER_COPY: Record<GatewayLifecycleAction, string> = {
  setup: 'Setting up OpenClaw...',
  start: 'Starting gateway...',
  stop: 'Stopping gateway...',
  restart: 'Restarting gateway...',
  reconnect: 'Restoring gateway connection...',
}

const CONTROL_PLANE_COPY: Record<
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

const FALLBACK_CONTROL_PLANE_COPY = {
  badgeVariant: 'outline' as const,
  badgeLabel: 'Unknown',
  title: 'Gateway State Unknown',
  description:
    'BrowserOS received a gateway status it does not recognize yet. Refreshing or reconnecting should restore a known state.',
}

const RECOVERY_REASON_COPY: Record<
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

const StatusBadge: FC<{ status: OpenClawStatus['status'] }> = ({ status }) => {
  const variants: Record<
    OpenClawStatus['status'],
    {
      variant: 'default' | 'secondary' | 'outline' | 'destructive'
      label: string
    }
  > = {
    running: { variant: 'default', label: 'Running' },
    starting: { variant: 'secondary', label: 'Starting...' },
    stopped: { variant: 'outline', label: 'Stopped' },
    error: { variant: 'destructive', label: 'Error' },
    uninitialized: { variant: 'outline', label: 'Not Set Up' },
  }
  const current = variants[status] ?? {
    variant: 'outline' as const,
    label: 'Unknown',
  }
  return <Badge variant={current.variant}>{current.label}</Badge>
}

const ControlPlaneBadge: FC<{
  status: OpenClawStatus['controlPlaneStatus']
}> = ({ status }) => {
  const current = CONTROL_PLANE_COPY[status] ?? FALLBACK_CONTROL_PLANE_COPY
  return <Badge variant={current.badgeVariant}>{current.badgeLabel}</Badge>
}

function getControlPlaneCopy(status: OpenClawStatus['controlPlaneStatus']) {
  return CONTROL_PLANE_COPY[status] ?? FALLBACK_CONTROL_PLANE_COPY
}

function getRecoveryDetail(status: OpenClawStatus): string | null {
  if (!status.lastRecoveryReason && !status.lastGatewayError) return null

  const detail = status.lastRecoveryReason
    ? RECOVERY_REASON_COPY[status.lastRecoveryReason]
    : null

  if (status.lastGatewayError && detail) {
    return `${detail} Latest gateway error: ${status.lastGatewayError}`
  }

  return status.lastGatewayError ?? detail
}

interface ProviderSelectorProps {
  providers: Array<{
    id: string
    type: string
    name: string
    modelId: string
    baseUrl?: string
  }>
  defaultProviderId: string
  selectedId: string
  onSelect: (id: string) => void
}

const ProviderSelector: FC<ProviderSelectorProps> = ({
  providers,
  defaultProviderId,
  selectedId,
  onSelect,
}) => {
  if (providers.length === 0) {
    return (
      <div className="space-y-2">
        <p className="font-medium text-sm">LLM Provider</p>
        <p className="text-muted-foreground text-sm">
          No compatible LLM providers configured.{' '}
          <a href="#/settings/ai" className="underline">
            Add one in AI settings
          </a>{' '}
          first.
        </p>
      </div>
    )
  }

  return (
    <div className="space-y-2">
      <label className="font-medium text-sm" htmlFor="provider-select">
        LLM Provider
      </label>
      <Select value={selectedId} onValueChange={onSelect}>
        <SelectTrigger id="provider-select">
          <SelectValue placeholder="Select a provider" />
        </SelectTrigger>
        <SelectContent>
          {providers.map((provider) => (
            <SelectItem key={provider.id} value={provider.id}>
              {provider.name} — {provider.modelId}
              {provider.id === defaultProviderId ? ' (default)' : ''}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <p className="text-muted-foreground text-xs">
        Uses your existing API key from BrowserOS settings. The key is passed to
        the container and never leaves your machine.
      </p>
    </div>
  )
}

interface AgentsPageHeaderProps {
  actionInProgress: boolean
  canManageAgents: boolean
  controlPlaneBusy: boolean
  reconnecting: boolean
  status: OpenClawStatus | null
  onCreateAgent: () => void
  onOpenTerminal: () => void
  onReconnect: () => void
  onRestart: () => void
  onStop: () => void
}

const AgentsPageHeader: FC<AgentsPageHeaderProps> = ({
  actionInProgress,
  canManageAgents,
  controlPlaneBusy,
  reconnecting,
  status,
  onCreateAgent,
  onOpenTerminal,
  onReconnect,
  onRestart,
  onStop,
}) => (
  <div className="flex items-center justify-between">
    <div>
      <h1 className="font-bold text-2xl">Agents</h1>
      <p className="text-muted-foreground text-sm">
        OpenClaw agents running in a local container
      </p>
    </div>

    {status && (
      <div className="flex items-center gap-2">
        <StatusBadge status={status.status} />
        {status.status !== 'uninitialized' && (
          <ControlPlaneBadge status={status.controlPlaneStatus} />
        )}

        {status.status === 'running' && (
          <>
            {status.controlPlaneStatus !== 'connected' && (
              <Button
                variant="outline"
                onClick={onReconnect}
                disabled={actionInProgress || controlPlaneBusy}
              >
                {reconnecting ? (
                  <Loader2 className="mr-2 size-4 animate-spin" />
                ) : (
                  <RefreshCw className="mr-2 size-4" />
                )}
                Retry Connection
              </Button>
            )}
            <Button
              variant="ghost"
              size="icon"
              onClick={onRestart}
              disabled={actionInProgress}
              title="Restart gateway"
            >
              <RefreshCw className="size-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              onClick={onStop}
              disabled={actionInProgress}
              title="Stop gateway"
            >
              <Square className="size-4" />
            </Button>
            <Button variant="outline" onClick={onOpenTerminal}>
              <TerminalSquare className="mr-1 size-4" />
              Terminal
            </Button>
            <Button onClick={onCreateAgent} disabled={!canManageAgents}>
              <Plus className="mr-1 size-4" />
              New Agent
            </Button>
          </>
        )}
      </div>
    )}
  </div>
)

function LifecycleAlert({ message }: { message: string }) {
  return (
    <Alert>
      <Loader2 className="animate-spin" />
      <AlertTitle>{message}</AlertTitle>
    </Alert>
  )
}

function InlineErrorAlert({
  message,
  onDismiss,
}: {
  message: string
  onDismiss: () => void
}) {
  return (
    <Alert variant="destructive">
      <AlertCircle />
      <AlertTitle>OpenClaw action failed</AlertTitle>
      <AlertDescription>
        <p>{message}</p>
        <div className="mt-2">
          <Button variant="outline" size="sm" onClick={onDismiss}>
            Dismiss
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  )
}

interface ControlPlaneAlertProps {
  actionInProgress: boolean
  controlPlaneBusy: boolean
  controlPlaneCopy: ReturnType<typeof getControlPlaneCopy>
  reconnecting: boolean
  recoveryDetail: string | null
  status: OpenClawStatus
  onReconnect: () => void
  onRestart: () => void
}

const ControlPlaneAlert: FC<ControlPlaneAlertProps> = ({
  actionInProgress,
  controlPlaneBusy,
  controlPlaneCopy,
  reconnecting,
  recoveryDetail,
  status,
  onReconnect,
  onRestart,
}) => (
  <Alert
    variant={status.controlPlaneStatus === 'failed' ? 'destructive' : 'default'}
  >
    {status.controlPlaneStatus === 'failed' ? (
      <ShieldAlert />
    ) : status.controlPlaneStatus === 'recovering' ? (
      <Wrench />
    ) : (
      <WifiOff />
    )}
    <AlertTitle>{controlPlaneCopy.title}</AlertTitle>
    <AlertDescription>
      <p>{controlPlaneCopy.description}</p>
      {recoveryDetail && <p>{recoveryDetail}</p>}
      <div className="mt-2 flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={onReconnect}
          disabled={actionInProgress || controlPlaneBusy}
        >
          {reconnecting ? (
            <Loader2 className="mr-2 size-4 animate-spin" />
          ) : (
            <RefreshCw className="mr-2 size-4" />
          )}
          Retry Connection
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={onRestart}
          disabled={actionInProgress}
        >
          Restart Gateway
        </Button>
      </div>
    </AlertDescription>
  </Alert>
)

interface GatewayStateCardsProps {
  actionInProgress: boolean
  status: OpenClawStatus | null
  onOpenSetup: () => void
  onRestart: () => void
  onStart: () => void
}

const GatewayStateCards: FC<GatewayStateCardsProps> = ({
  actionInProgress,
  status,
  onOpenSetup,
  onRestart,
  onStart,
}) => (
  <>
    {status?.status === 'uninitialized' && (
      <Card>
        <CardContent className="flex flex-col items-center gap-4 py-12">
          <Cpu className="size-12 text-muted-foreground" />
          <div className="text-center">
            <h3 className="font-semibold text-lg">Set Up OpenClaw</h3>
            <p className="text-muted-foreground text-sm">
              {status.podmanAvailable
                ? 'Create a local BrowserOS VM to run autonomous agents with full tool access.'
                : 'BrowserOS VM runtime is unavailable on this system.'}
            </p>
          </div>
          {status.podmanAvailable && (
            <Button onClick={onOpenSetup}>Set Up Now</Button>
          )}
        </CardContent>
      </Card>
    )}

    {status?.status === 'stopped' && (
      <Card>
        <CardContent className="flex flex-col items-center gap-4 py-12">
          <Cpu className="size-12 text-muted-foreground" />
          <div className="text-center">
            <h3 className="font-semibold text-lg">Gateway Stopped</h3>
            <p className="text-muted-foreground text-sm">
              The OpenClaw gateway is not running.
            </p>
          </div>
          <Button onClick={onStart} disabled={actionInProgress}>
            Start Gateway
          </Button>
        </CardContent>
      </Card>
    )}

    {status?.status === 'error' && (
      <Card className="border-destructive">
        <CardContent className="flex flex-col items-center gap-4 py-12">
          <AlertCircle className="size-12 text-destructive" />
          <div className="text-center">
            <h3 className="font-semibold text-lg">Gateway Error</h3>
            <p className="text-muted-foreground text-sm">
              {status.error ?? status.lastGatewayError}
            </p>
          </div>
          <div className="flex gap-2">
            <Button onClick={onStart} disabled={actionInProgress}>
              Start Gateway
            </Button>
            <Button
              variant="outline"
              onClick={onRestart}
              disabled={actionInProgress}
            >
              Restart Gateway
            </Button>
          </div>
        </CardContent>
      </Card>
    )}
  </>
)

interface RunningAgentsSectionProps {
  agents: AgentEntry[]
  agentsLoading: boolean
  canManageAgents: boolean
  deleting: boolean
  status: OpenClawStatus | null
  onChatAgent: (agentId: string) => void
  onCreateAgent: () => void
  onDeleteAgent: (agentId: string) => void
}

const RunningAgentsSection: FC<RunningAgentsSectionProps> = ({
  agents,
  agentsLoading,
  canManageAgents,
  deleting,
  status,
  onChatAgent,
  onCreateAgent,
  onDeleteAgent,
}) => {
  if (status?.status !== 'running') return null

  if (agentsLoading) {
    return (
      <div className="flex justify-center py-8">
        <Loader2 className="size-5 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (agents.length === 0) {
    return (
      <Card>
        <CardContent className="flex flex-col items-center gap-3 py-8">
          <p className="text-muted-foreground text-sm">
            No agents yet. Create one to get started.
          </p>
          <Button
            variant="outline"
            onClick={onCreateAgent}
            disabled={!canManageAgents}
          >
            <Plus className="mr-1 size-4" />
            Create Agent
          </Button>
        </CardContent>
      </Card>
    )
  }

  return (
    <div className="space-y-3">
      {agents.map((agent) => (
        <Card key={agent.agentId}>
          <CardHeader className="flex flex-row items-center justify-between py-3">
            <div className="flex items-center gap-3">
              <Cpu className="size-5 text-muted-foreground" />
              <div>
                <div className="flex items-center gap-2">
                  <CardTitle className="text-base">{agent.name}</CardTitle>
                </div>
                <p className="font-mono text-muted-foreground text-xs">
                  {agent.workspace}
                </p>
              </div>
            </div>
            <div className="flex items-center gap-1">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => onChatAgent(agent.agentId)}
                disabled={!canManageAgents}
              >
                <MessageSquare className="mr-1 size-4" />
                Chat
              </Button>
              {agent.agentId !== 'main' && (
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => onDeleteAgent(agent.agentId)}
                  disabled={!canManageAgents || deleting}
                >
                  <Trash2 className="size-4 text-destructive" />
                </Button>
              )}
            </div>
          </CardHeader>
        </Card>
      ))}
    </div>
  )
}

export const AgentsPage: FC = () => {
  const navigate = useNavigate()
  const {
    status,
    loading: statusLoading,
    error: statusError,
  } = useOpenClawStatus()
  const { providers, defaultProviderId } = useLlmProviders()
  const agentsQueryEnabled =
    status?.status === 'running' && status.controlPlaneStatus === 'connected'
  const {
    agents,
    loading: agentsLoading,
    error: agentsError,
  } = useOpenClawAgents(agentsQueryEnabled)
  const {
    setupOpenClaw,
    createAgent,
    deleteAgent,
    startOpenClaw,
    stopOpenClaw,
    restartOpenClaw,
    reconnectOpenClaw,
    actionInProgress,
    settingUp,
    creating,
    deleting,
    reconnecting,
    pendingGatewayAction,
  } = useOpenClawMutations()

  const [setupOpen, setSetupOpen] = useState(false)
  const [setupProviderId, setSetupProviderId] = useState('')
  const [createOpen, setCreateOpen] = useState(false)
  const [newName, setNewName] = useState('')
  const [createProviderId, setCreateProviderId] = useState('')

  const [showTerminal, setShowTerminal] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const compatibleProviders = getOpenClawSupportedProviders(providers)

  useEffect(() => {
    if (compatibleProviders.length === 0) return
    const fallbackId =
      compatibleProviders.find((provider) => provider.id === defaultProviderId)
        ?.id ?? compatibleProviders[0].id

    if (setupOpen && !setupProviderId) setSetupProviderId(fallbackId)
    if (createOpen && !createProviderId) setCreateProviderId(fallbackId)
  }, [
    setupOpen,
    createOpen,
    setupProviderId,
    createProviderId,
    compatibleProviders,
    defaultProviderId,
  ])

  useEffect(() => {
    if (!createOpen) return
    setNewName((current) => current || 'agent')
  }, [createOpen])

  const lifecyclePending = pendingGatewayAction !== null
  const inlineError = lifecyclePending
    ? null
    : (error ?? statusError?.message ?? agentsError?.message ?? null)
  const lifecycleBanner = pendingGatewayAction
    ? LIFECYCLE_BANNER_COPY[pendingGatewayAction]
    : null

  const gatewayUiState = useMemo(() => {
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

    const canManageAgents =
      status.status === 'running' && status.controlPlaneStatus === 'connected'

    const controlPlaneDegraded =
      status.status === 'running' && status.controlPlaneStatus !== 'connected'

    return {
      canManageAgents,
      controlPlaneBusy,
      controlPlaneDegraded,
    }
  }, [status])

  const canManageAgents = gatewayUiState.canManageAgents && !lifecyclePending
  const showControlPlaneDegraded =
    !lifecyclePending && gatewayUiState.controlPlaneDegraded

  const recoveryDetail = status ? getRecoveryDetail(status) : null
  const controlPlaneCopy = status
    ? getControlPlaneCopy(status.controlPlaneStatus)
    : FALLBACK_CONTROL_PLANE_COPY

  const runWithErrorHandling = async (fn: () => Promise<unknown>) => {
    setError(null)
    try {
      await fn()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleSetup = async () => {
    const provider = compatibleProviders.find(
      (item) => item.id === setupProviderId,
    )

    await runWithErrorHandling(async () => {
      await setupOpenClaw({
        providerType: provider?.type,
        providerName: provider?.name,
        baseUrl: provider?.baseUrl,
        apiKey: provider?.apiKey,
        modelId: provider?.modelId,
      })
      setSetupOpen(false)
    })
  }

  const handleCreate = async () => {
    if (!newName.trim()) return
    const provider = compatibleProviders.find(
      (item) => item.id === createProviderId,
    )
    const normalizedName = newName.trim().toLowerCase().replace(/\s+/g, '-')

    await runWithErrorHandling(async () => {
      await createAgent({
        name: normalizedName,
        providerType: provider?.type,
        providerName: provider?.name,
        baseUrl: provider?.baseUrl,
        apiKey: provider?.apiKey,
        modelId: provider?.modelId,
      })
      setCreateOpen(false)
      setNewName('')
    })
  }

  const handleDelete = async (id: string) => {
    await runWithErrorHandling(async () => {
      await deleteAgent(id)
    })
  }

  const handleStop = async () => {
    await runWithErrorHandling(async () => {
      await stopOpenClaw()
    })
  }

  const handleStart = async () => {
    await runWithErrorHandling(async () => {
      await startOpenClaw()
    })
  }

  const handleRestart = async () => {
    await runWithErrorHandling(async () => {
      await restartOpenClaw()
    })
  }

  const handleReconnect = async () => {
    await runWithErrorHandling(async () => {
      await reconnectOpenClaw()
    })
  }

  if (showTerminal) {
    return <AgentTerminal onBack={() => setShowTerminal(false)} />
  }

  if (statusLoading && !status) {
    return (
      <div className="flex items-center justify-center py-20">
        <Loader2 className="size-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <div className="fade-in slide-in-from-bottom-5 animate-in space-y-6 duration-500">
      <AgentsPageHeader
        actionInProgress={actionInProgress}
        canManageAgents={canManageAgents}
        controlPlaneBusy={gatewayUiState.controlPlaneBusy}
        reconnecting={reconnecting}
        status={status}
        onCreateAgent={() => setCreateOpen(true)}
        onOpenTerminal={() => setShowTerminal(true)}
        onReconnect={handleReconnect}
        onRestart={handleRestart}
        onStop={handleStop}
      />

      {lifecycleBanner && <LifecycleAlert message={lifecycleBanner} />}

      {inlineError && (
        <InlineErrorAlert
          message={inlineError}
          onDismiss={() => setError(null)}
        />
      )}

      {status && showControlPlaneDegraded && (
        <ControlPlaneAlert
          actionInProgress={actionInProgress}
          controlPlaneBusy={gatewayUiState.controlPlaneBusy}
          controlPlaneCopy={controlPlaneCopy}
          reconnecting={reconnecting}
          recoveryDetail={recoveryDetail}
          status={status}
          onReconnect={handleReconnect}
          onRestart={handleRestart}
        />
      )}

      <GatewayStateCards
        actionInProgress={actionInProgress}
        status={status}
        onOpenSetup={() => setSetupOpen(true)}
        onRestart={handleRestart}
        onStart={handleStart}
      />

      <RunningAgentsSection
        agents={agents}
        agentsLoading={agentsLoading}
        canManageAgents={canManageAgents}
        deleting={deleting}
        status={status}
        onChatAgent={(agentId) => navigate(`/agents/${agentId}`)}
        onCreateAgent={() => setCreateOpen(true)}
        onDeleteAgent={(agentId) => {
          void handleDelete(agentId)
        }}
      />

      <Dialog open={setupOpen} onOpenChange={setSetupOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Set Up OpenClaw</DialogTitle>
          </DialogHeader>
          <div className="space-y-4 py-2">
            <ProviderSelector
              providers={compatibleProviders}
              defaultProviderId={defaultProviderId}
              selectedId={setupProviderId}
              onSelect={setSetupProviderId}
            />
            <Button
              onClick={handleSetup}
              disabled={settingUp || compatibleProviders.length === 0}
              className="w-full"
            >
              {settingUp ? (
                <>
                  <Loader2 className="mr-2 size-4 animate-spin" />
                  Setting up...
                </>
              ) : (
                'Set Up & Start'
              )}
            </Button>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create Agent</DialogTitle>
          </DialogHeader>
          <div className="space-y-4 py-2">
            <div>
              <label
                htmlFor="agent-name"
                className="mb-1 block font-medium text-sm"
              >
                Agent Name
              </label>
              <Input
                id="agent-name"
                value={newName}
                onChange={(event) => setNewName(event.target.value)}
                placeholder="research-agent"
                onKeyDown={(event) => {
                  if (event.key === 'Enter') void handleCreate()
                }}
              />
              <p className="mt-1 text-muted-foreground text-xs">
                Lowercase letters, numbers, and hyphens only.
              </p>
            </div>

            <ProviderSelector
              providers={compatibleProviders}
              defaultProviderId={defaultProviderId}
              selectedId={createProviderId}
              onSelect={setCreateProviderId}
            />

            <Button
              onClick={handleCreate}
              disabled={
                !newName.trim() ||
                creating ||
                !canManageAgents ||
                compatibleProviders.length === 0
              }
              className="w-full"
            >
              {creating ? (
                <>
                  <Loader2 className="mr-2 size-4 animate-spin" />
                  Creating...
                </>
              ) : (
                'Create Agent'
              )}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
