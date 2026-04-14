import type {
  BrowserOSCustomRoleInput,
  BrowserOSRoleBoundary,
} from '@browseros/shared/types/role-aware-agents'
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
import { Textarea } from '@/components/ui/textarea'
import { useLlmProviders } from '@/lib/llm-providers/useLlmProviders'
import { AgentChat } from './AgentChat'
import { AgentTerminal } from './AgentTerminal'
import {
  type AgentEntry,
  type OpenClawStatus,
  type RoleTemplateSummary,
  useOpenClawAgents,
  useOpenClawMutations,
  useOpenClawRoles,
  useOpenClawStatus,
} from './useOpenClaw'

const OAUTH_ONLY_TYPES = new Set(['chatgpt-pro', 'github-copilot', 'qwen-code'])
const CUSTOM_ROLE_VALUE = '__custom__'
const PLAIN_AGENT_VALUE = '__plain__'
type AgentCreationMode = 'builtin' | 'custom' | 'plain'

function createDefaultCustomRoleBoundaries(): BrowserOSRoleBoundary[] {
  return [
    {
      key: 'draft-external-comms',
      label: 'Draft external communications',
      description: 'May prepare outbound messages for review.',
      defaultMode: 'allow',
    },
    {
      key: 'send-external-comms',
      label: 'Send external communications',
      description: 'Should require approval before sending messages.',
      defaultMode: 'ask',
    },
    {
      key: 'calendar-mutations',
      label: 'Modify calendar events',
      description: 'Should ask before moving or creating calendar events.',
      defaultMode: 'ask',
    },
  ]
}

function parseCommaSeparatedList(input: string): string[] {
  return input
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean)
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

export const AgentsPage: FC = () => {
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
  const { roles, loading: rolesLoading, error: rolesError } = useOpenClawRoles()
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
  } = useOpenClawMutations()

  const [setupOpen, setSetupOpen] = useState(false)
  const [setupProviderId, setSetupProviderId] = useState('')

  const [createOpen, setCreateOpen] = useState(false)
  const [selectedRoleValue, setSelectedRoleValue] = useState<
    | RoleTemplateSummary['id']
    | typeof CUSTOM_ROLE_VALUE
    | typeof PLAIN_AGENT_VALUE
  >('chief-of-staff')
  const [newName, setNewName] = useState('')
  const [createProviderId, setCreateProviderId] = useState('')
  const [customRole, setCustomRole] = useState<BrowserOSCustomRoleInput>({
    name: '',
    shortDescription: '',
    longDescription: '',
    recommendedApps: [],
    boundaries: createDefaultCustomRoleBoundaries(),
  })

  const [chatAgent, setChatAgent] = useState<AgentEntry | null>(null)
  const [showTerminal, setShowTerminal] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const compatibleProviders = providers.filter(
    (provider) => provider.apiKey && !OAUTH_ONLY_TYPES.has(provider.type),
  )
  const creationMode: AgentCreationMode =
    selectedRoleValue === CUSTOM_ROLE_VALUE
      ? 'custom'
      : selectedRoleValue === PLAIN_AGENT_VALUE
        ? 'plain'
        : 'builtin'
  const isCustomRole = creationMode === 'custom'
  const isPlainAgent = creationMode === 'plain'
  const selectedRole =
    creationMode === 'builtin'
      ? (roles.find((role) => role.id === selectedRoleValue) ??
        roles[0] ??
        null)
      : null

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
    if (!createOpen || roles.length === 0) return

    const defaultRole = roles.find((role) => role.id === 'chief-of-staff')
    const nextRole = defaultRole ?? roles[0]

    setSelectedRoleValue((current) => {
      if (current === CUSTOM_ROLE_VALUE || current === PLAIN_AGENT_VALUE)
        return current
      const hasCurrent = roles.some((role) => role.id === current)
      return hasCurrent ? current : nextRole.id
    })
    setNewName((current) => current || nextRole.defaultAgentName)
  }, [createOpen, roles])

  useEffect(() => {
    if (!createOpen) return

    if (isCustomRole) {
      setNewName(
        (current) =>
          current || customRole.name.trim().toLowerCase().replace(/\s+/g, '-'),
      )
      return
    }

    if (isPlainAgent) {
      setNewName((current) => current || 'agent')
      return
    }

    if (selectedRole) {
      setNewName((current) => current || selectedRole.defaultAgentName)
    }
  }, [createOpen, isCustomRole, isPlainAgent, customRole.name, selectedRole])

  const inlineError =
    error ??
    statusError?.message ??
    agentsError?.message ??
    rolesError?.message ??
    null

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

  const recoveryDetail = status ? getRecoveryDetail(status) : null
  const controlPlaneCopy = status
    ? getControlPlaneCopy(status.controlPlaneStatus)
    : null

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
    const customRolePayload = isCustomRole
      ? {
          ...customRole,
          name: customRole.name.trim(),
          shortDescription: customRole.shortDescription.trim(),
          longDescription: customRole.longDescription.trim(),
        }
      : undefined

    if (
      isCustomRole &&
      (!customRolePayload?.name ||
        !customRolePayload.shortDescription ||
        !customRolePayload.longDescription)
    ) {
      setError(
        'Custom roles require a role name, short description, and long description.',
      )
      return
    }

    if (creationMode === 'builtin' && !selectedRole) return

    await runWithErrorHandling(async () => {
      await createAgent({
        name: normalizedName,
        roleId: creationMode === 'builtin' ? selectedRole?.id : undefined,
        customRole: isCustomRole ? customRolePayload : undefined,
        providerType: provider?.type,
        providerName: provider?.name,
        baseUrl: provider?.baseUrl,
        apiKey: provider?.apiKey,
        modelId: provider?.modelId,
      })
      setCreateOpen(false)
      setNewName('')
      setCustomRole({
        name: '',
        shortDescription: '',
        longDescription: '',
        recommendedApps: [],
        boundaries: createDefaultCustomRoleBoundaries(),
      })
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

  if (chatAgent) {
    return (
      <AgentChat
        agentId={chatAgent.agentId}
        agentName={chatAgent.name}
        onBack={() => setChatAgent(null)}
      />
    )
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
                    onClick={handleReconnect}
                    disabled={
                      actionInProgress || gatewayUiState.controlPlaneBusy
                    }
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
                  onClick={handleRestart}
                  disabled={actionInProgress}
                  title="Restart gateway"
                >
                  <RefreshCw className="size-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={handleStop}
                  disabled={actionInProgress}
                  title="Stop gateway"
                >
                  <Square className="size-4" />
                </Button>
                <Button variant="outline" onClick={() => setShowTerminal(true)}>
                  <TerminalSquare className="mr-1 size-4" />
                  Terminal
                </Button>
                <Button
                  onClick={() => setCreateOpen(true)}
                  disabled={!gatewayUiState.canManageAgents}
                >
                  <Plus className="mr-1 size-4" />
                  New Agent
                </Button>
              </>
            )}
          </div>
        )}
      </div>

      {inlineError && (
        <Alert variant="destructive">
          <AlertCircle />
          <AlertTitle>OpenClaw action failed</AlertTitle>
          <AlertDescription>
            <p>{inlineError}</p>
            <div className="mt-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => setError(null)}
              >
                Dismiss
              </Button>
            </div>
          </AlertDescription>
        </Alert>
      )}

      {status && gatewayUiState.controlPlaneDegraded && (
        <Alert
          variant={
            status.controlPlaneStatus === 'failed' ? 'destructive' : 'default'
          }
        >
          {status.controlPlaneStatus === 'failed' ? (
            <ShieldAlert />
          ) : status.controlPlaneStatus === 'recovering' ? (
            <Wrench />
          ) : (
            <WifiOff />
          )}
          <AlertTitle>{controlPlaneCopy?.title}</AlertTitle>
          <AlertDescription>
            <p>{controlPlaneCopy?.description}</p>
            {recoveryDetail && <p>{recoveryDetail}</p>}
            <div className="mt-2 flex flex-wrap gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={handleReconnect}
                disabled={actionInProgress || gatewayUiState.controlPlaneBusy}
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
                onClick={handleRestart}
                disabled={actionInProgress}
              >
                Restart Gateway
              </Button>
            </div>
          </AlertDescription>
        </Alert>
      )}

      {status?.status === 'uninitialized' && (
        <Card>
          <CardContent className="flex flex-col items-center gap-4 py-12">
            <Cpu className="size-12 text-muted-foreground" />
            <div className="text-center">
              <h3 className="font-semibold text-lg">Set Up OpenClaw</h3>
              <p className="text-muted-foreground text-sm">
                {status.podmanAvailable
                  ? 'Create a local container to run autonomous agents with full tool access.'
                  : 'Podman is required to run OpenClaw agents. Install Podman first.'}
              </p>
            </div>
            {status.podmanAvailable && (
              <Button onClick={() => setSetupOpen(true)}>Set Up Now</Button>
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
            <Button onClick={handleStart} disabled={actionInProgress}>
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
              <Button onClick={handleStart} disabled={actionInProgress}>
                Start Gateway
              </Button>
              <Button
                variant="outline"
                onClick={handleRestart}
                disabled={actionInProgress}
              >
                Restart Gateway
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {status?.status === 'running' && (
        <div className="space-y-3">
          {agentsLoading ? (
            <div className="flex justify-center py-8">
              <Loader2 className="size-5 animate-spin text-muted-foreground" />
            </div>
          ) : agents.length === 0 ? (
            <Card>
              <CardContent className="flex flex-col items-center gap-3 py-8">
                <p className="text-muted-foreground text-sm">
                  No agents yet. Create one to get started.
                </p>
                <Button
                  variant="outline"
                  onClick={() => setCreateOpen(true)}
                  disabled={!gatewayUiState.canManageAgents}
                >
                  <Plus className="mr-1 size-4" />
                  Create Agent
                </Button>
              </CardContent>
            </Card>
          ) : (
            agents.map((agent) => (
              <Card key={agent.agentId}>
                <CardHeader className="flex flex-row items-center justify-between py-3">
                  <div className="flex items-center gap-3">
                    <Cpu className="size-5 text-muted-foreground" />
                    <div>
                      <div className="flex items-center gap-2">
                        <CardTitle className="text-base">
                          {agent.name}
                        </CardTitle>
                        {agent.role && (
                          <Badge variant="secondary">
                            {agent.role.roleName}
                          </Badge>
                        )}
                      </div>
                      <p className="font-mono text-muted-foreground text-xs">
                        {agent.workspace}
                      </p>
                      {agent.role && (
                        <p className="text-muted-foreground text-xs">
                          {agent.role.shortDescription}
                        </p>
                      )}
                    </div>
                  </div>
                  <div className="flex items-center gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setChatAgent(agent)}
                      disabled={!gatewayUiState.canManageAgents}
                    >
                      <MessageSquare className="mr-1 size-4" />
                      Chat
                    </Button>
                    {agent.agentId !== 'main' && (
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => handleDelete(agent.agentId)}
                        disabled={!gatewayUiState.canManageAgents || deleting}
                      >
                        <Trash2 className="size-4 text-destructive" />
                      </Button>
                    )}
                  </div>
                </CardHeader>
              </Card>
            ))
          )}
        </div>
      )}

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
            <div className="space-y-2">
              <label className="font-medium text-sm" htmlFor="agent-role">
                Agent Role
              </label>
              <Select
                value={selectedRoleValue}
                onValueChange={(value) => {
                  if (value === CUSTOM_ROLE_VALUE) {
                    setSelectedRoleValue(CUSTOM_ROLE_VALUE)
                    setNewName(
                      customRole.name
                        .trim()
                        .toLowerCase()
                        .replace(/\s+/g, '-') || 'custom-agent',
                    )
                    return
                  }

                  if (value === PLAIN_AGENT_VALUE) {
                    setSelectedRoleValue(PLAIN_AGENT_VALUE)
                    setNewName('agent')
                    return
                  }

                  const role = roles.find((item) => item.id === value)
                  if (!role) return

                  setSelectedRoleValue(role.id)
                  setNewName(role.defaultAgentName)
                }}
                disabled={rolesLoading}
              >
                <SelectTrigger id="agent-role">
                  <SelectValue
                    placeholder={
                      rolesLoading ? 'Loading roles...' : 'Select a role'
                    }
                  />
                </SelectTrigger>
                <SelectContent>
                  {roles.map((role) => (
                    <SelectItem key={role.id} value={role.id}>
                      {role.name}
                    </SelectItem>
                  ))}
                  <SelectItem value={PLAIN_AGENT_VALUE}>Plain Agent</SelectItem>
                  <SelectItem value={CUSTOM_ROLE_VALUE}>Custom Role</SelectItem>
                </SelectContent>
              </Select>
              {selectedRole && !isCustomRole && (
                <Card>
                  <CardContent className="space-y-3 py-4">
                    <div>
                      <div className="font-medium text-sm">
                        {selectedRole.name}
                      </div>
                      <p className="text-muted-foreground text-xs">
                        {selectedRole.shortDescription}
                      </p>
                    </div>
                    <div>
                      <div className="font-medium text-xs">
                        Recommended Apps
                      </div>
                      <p className="text-muted-foreground text-xs">
                        {selectedRole.recommendedApps.join(', ')}
                      </p>
                    </div>
                    <div>
                      <div className="font-medium text-xs">
                        Default Boundaries
                      </div>
                      <ul className="space-y-1 text-muted-foreground text-xs">
                        {selectedRole.boundaries.map((boundary) => (
                          <li key={boundary.key}>
                            {boundary.label}: {boundary.defaultMode}
                          </li>
                        ))}
                      </ul>
                    </div>
                  </CardContent>
                </Card>
              )}
              {isPlainAgent && (
                <Card>
                  <CardContent className="space-y-2 py-4">
                    <div className="font-medium text-sm">Plain Agent</div>
                    <p className="text-muted-foreground text-xs">
                      No role bootstrap or defaults. Intended for temporary
                      development and testing only.
                    </p>
                  </CardContent>
                </Card>
              )}
            </div>

            {isCustomRole && (
              <Card>
                <CardContent className="space-y-4 py-4">
                  <div className="space-y-2">
                    <label
                      htmlFor="custom-role-name"
                      className="font-medium text-sm"
                    >
                      Custom Role Name
                    </label>
                    <Input
                      id="custom-role-name"
                      value={customRole.name}
                      onChange={(event) => {
                        const name = event.target.value
                        setCustomRole((current) => ({ ...current, name }))
                        setNewName(
                          name.trim().toLowerCase().replace(/\s+/g, '-') ||
                            'custom-agent',
                        )
                      }}
                      placeholder="Board Prep Operator"
                    />
                  </div>
                  <div className="space-y-2">
                    <label
                      htmlFor="custom-role-short-description"
                      className="font-medium text-sm"
                    >
                      Short Description
                    </label>
                    <Input
                      id="custom-role-short-description"
                      value={customRole.shortDescription}
                      onChange={(event) =>
                        setCustomRole((current) => ({
                          ...current,
                          shortDescription: event.target.value,
                        }))
                      }
                      placeholder="Prepares executive briefs and weekly follow-ups."
                    />
                  </div>
                  <div className="space-y-2">
                    <label
                      htmlFor="custom-role-long-description"
                      className="font-medium text-sm"
                    >
                      Long Description
                    </label>
                    <Textarea
                      id="custom-role-long-description"
                      value={customRole.longDescription}
                      onChange={(event) =>
                        setCustomRole((current) => ({
                          ...current,
                          longDescription: event.target.value,
                        }))
                      }
                      placeholder="Describe the role, purpose, and what kinds of outcomes this agent should produce."
                      rows={4}
                    />
                  </div>
                  <div className="space-y-2">
                    <label
                      htmlFor="custom-role-apps"
                      className="font-medium text-sm"
                    >
                      Recommended Apps
                    </label>
                    <Input
                      id="custom-role-apps"
                      value={customRole.recommendedApps.join(', ')}
                      onChange={(event) =>
                        setCustomRole((current) => ({
                          ...current,
                          recommendedApps: parseCommaSeparatedList(
                            event.target.value,
                          ),
                        }))
                      }
                      placeholder="gmail, slack, notion"
                    />
                    <p className="text-muted-foreground text-xs">
                      Comma-separated. Used as role guidance only in this
                      milestone.
                    </p>
                  </div>
                  <div className="space-y-3">
                    <div>
                      <div className="font-medium text-sm">
                        Boundary Defaults
                      </div>
                      <p className="text-muted-foreground text-xs">
                        Set the starting behavior for common high-impact
                        actions.
                      </p>
                    </div>
                    {customRole.boundaries.map((boundary) => (
                      <div
                        key={boundary.key}
                        className="grid gap-2 rounded-lg border p-3"
                      >
                        <div>
                          <div className="font-medium text-sm">
                            {boundary.label}
                          </div>
                          <p className="text-muted-foreground text-xs">
                            {boundary.description}
                          </p>
                        </div>
                        <Select
                          value={boundary.defaultMode}
                          onValueChange={(value) =>
                            setCustomRole((current) => ({
                              ...current,
                              boundaries: current.boundaries.map((item) =>
                                item.key === boundary.key
                                  ? {
                                      ...item,
                                      defaultMode:
                                        value as BrowserOSRoleBoundary['defaultMode'],
                                    }
                                  : item,
                              ),
                            }))
                          }
                        >
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="allow">Allow</SelectItem>
                            <SelectItem value="ask">Ask</SelectItem>
                            <SelectItem value="block">Block</SelectItem>
                          </SelectContent>
                        </Select>
                      </div>
                    ))}
                  </div>
                </CardContent>
              </Card>
            )}

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
                rolesLoading ||
                !gatewayUiState.canManageAgents ||
                compatibleProviders.length === 0 ||
                (creationMode === 'builtin' && !selectedRole)
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
