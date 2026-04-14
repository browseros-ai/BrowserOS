import type {
  BrowserOSAgentRoleId,
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
  Square,
  TerminalSquare,
  Trash2,
} from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
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
  type RoleTemplateSummary,
  useCreateOpenClawAgentMutation,
  useDeleteOpenClawAgentMutation,
  useInvalidateOpenClawQueries,
  useOpenClawAgents,
  useOpenClawRoles,
  useOpenClawStatus,
  useRestartOpenClawMutation,
  useSetupOpenClawMutation,
  useStartOpenClawMutation,
  useStopOpenClawMutation,
} from './useOpenClaw'

const OAUTH_ONLY_TYPES = new Set(['chatgpt-pro', 'github-copilot', 'qwen-code'])
const CUSTOM_ROLE_VALUE = '__custom__'

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

const StatusBadge: FC<{ status: string }> = ({ status }) => {
  const variants: Record<
    string,
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
  const v = variants[status] ?? { variant: 'outline' as const, label: status }
  return <Badge variant={v.variant}>{v.label}</Badge>
}

export const AgentsPage: FC = () => {
  const { status, loading: statusLoading } = useOpenClawStatus()
  const { providers, defaultProviderId } = useLlmProviders()
  const { agents, loading: agentsLoading } = useOpenClawAgents()
  const { roles, loading: rolesLoading } = useOpenClawRoles()
  const invalidateOpenClawQueries = useInvalidateOpenClawQueries()

  const [setupOpen, setSetupOpen] = useState(false)
  const [setupProviderId, setSetupProviderId] = useState('')

  const [createOpen, setCreateOpen] = useState(false)
  const [selectedRoleValue, setSelectedRoleValue] = useState<
    RoleTemplateSummary['id'] | typeof CUSTOM_ROLE_VALUE
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

  const setupMutation = useSetupOpenClawMutation()
  const createAgentMutation = useCreateOpenClawAgentMutation()
  const deleteAgentMutation = useDeleteOpenClawAgentMutation()
  const startMutation = useStartOpenClawMutation()
  const stopMutation = useStopOpenClawMutation()
  const restartMutation = useRestartOpenClawMutation()

  const compatibleProviders = providers.filter(
    (p) => p.apiKey && !OAUTH_ONLY_TYPES.has(p.type),
  )
  const isCustomRole = selectedRoleValue === CUSTOM_ROLE_VALUE
  const selectedRole = !isCustomRole
    ? (roles.find((role) => role.id === selectedRoleValue) ?? roles[0] ?? null)
    : null
  const actionInProgress =
    deleteAgentMutation.isPending ||
    startMutation.isPending ||
    stopMutation.isPending ||
    restartMutation.isPending

  // Pre-select default provider when dialogs open
  useEffect(() => {
    if (compatibleProviders.length === 0) return
    const fallbackId =
      compatibleProviders.find((p) => p.id === defaultProviderId)?.id ??
      compatibleProviders[0].id
    if (setupOpen) setSetupProviderId(fallbackId)
    if (createOpen) setCreateProviderId(fallbackId)
  }, [setupOpen, createOpen, compatibleProviders, defaultProviderId])

  useEffect(() => {
    if (!createOpen || roles.length === 0) return

    const defaultRole = roles.find((role) => role.id === 'chief-of-staff')
    const nextRole = defaultRole ?? roles[0]

    setSelectedRoleValue((current) => {
      if (current === CUSTOM_ROLE_VALUE) return current
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
    if (selectedRole) {
      setNewName((current) => current || selectedRole.defaultAgentName)
    }
  }, [createOpen, isCustomRole, customRole.name, selectedRole])

  const handleSetup = async () => {
    const provider = compatibleProviders.find((p) => p.id === setupProviderId)
    setError(null)
    try {
      await setupMutation.mutateAsync({
        providerType: provider?.type,
        providerName: provider?.name,
        baseUrl: provider?.baseUrl,
        apiKey: provider?.apiKey,
        modelId: provider?.modelId,
      })
      setSetupOpen(false)
      await invalidateOpenClawQueries()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleCreate = async () => {
    if (!newName.trim()) return
    const provider = compatibleProviders.find((p) => p.id === createProviderId)
    setError(null)
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
    if (!isCustomRole && !selectedRole) return

    try {
      await createAgentMutation.mutateAsync({
        name: normalizedName,
        roleId: !isCustomRole
          ? (selectedRole?.id as BrowserOSAgentRoleId)
          : undefined,
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
      await invalidateOpenClawQueries()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleDelete = async (id: string) => {
    try {
      await deleteAgentMutation.mutateAsync(id)
      await invalidateOpenClawQueries()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleStop = async () => {
    try {
      await stopMutation.mutateAsync()
      await invalidateOpenClawQueries()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleStart = async () => {
    setError(null)
    try {
      await startMutation.mutateAsync()
      await invalidateOpenClawQueries()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleRestart = async () => {
    try {
      await restartMutation.mutateAsync()
      await invalidateOpenClawQueries()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
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

  if (statusLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <Loader2 className="size-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <div className="fade-in slide-in-from-bottom-5 animate-in space-y-6 duration-500">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="font-bold text-2xl">Agents</h1>
          <p className="text-muted-foreground text-sm">
            OpenClaw agents running in a local container
          </p>
        </div>
        <div className="flex items-center gap-2">
          {status?.status === 'running' && (
            <>
              <StatusBadge status="running" />
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
              <Button onClick={() => setCreateOpen(true)}>
                <Plus className="mr-1 size-4" />
                New Agent
              </Button>
            </>
          )}
        </div>
      </div>

      {/* Error banner */}
      {error && (
        <Card className="border-destructive">
          <CardContent className="flex items-center gap-2 py-3">
            <AlertCircle className="size-4 text-destructive" />
            <p className="text-destructive text-sm">{error}</p>
            <Button
              variant="ghost"
              size="sm"
              className="ml-auto"
              onClick={() => setError(null)}
            >
              Dismiss
            </Button>
          </CardContent>
        </Card>
      )}

      {/* Uninitialized state */}
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

      {/* Stopped state */}
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

      {/* Error state */}
      {status?.status === 'error' && (
        <Card className="border-destructive">
          <CardContent className="flex flex-col items-center gap-4 py-12">
            <AlertCircle className="size-12 text-destructive" />
            <div className="text-center">
              <h3 className="font-semibold text-lg">Gateway Error</h3>
              <p className="text-muted-foreground text-sm">{status.error}</p>
            </div>
            <Button onClick={handleRestart} disabled={actionInProgress}>
              Retry
            </Button>
          </CardContent>
        </Card>
      )}

      {/* Agent list */}
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
                <Button variant="outline" onClick={() => setCreateOpen(true)}>
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
                    >
                      <MessageSquare className="mr-1 size-4" />
                      Chat
                    </Button>
                    {agent.agentId !== 'main' && (
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => handleDelete(agent.agentId)}
                        disabled={actionInProgress}
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

      {/* Setup Dialog (with provider selector) */}
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
              disabled={setupMutation.isPending}
              className="w-full"
            >
              {setupMutation.isPending ? (
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

      {/* Create Agent Dialog */}
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
                      onChange={(e) => {
                        const name = e.target.value
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
                      onChange={(e) =>
                        setCustomRole((current) => ({
                          ...current,
                          shortDescription: e.target.value,
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
                      onChange={(e) =>
                        setCustomRole((current) => ({
                          ...current,
                          longDescription: e.target.value,
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
                      onChange={(e) =>
                        setCustomRole((current) => ({
                          ...current,
                          recommendedApps: parseCommaSeparatedList(
                            e.target.value,
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
                onChange={(e) => setNewName(e.target.value)}
                placeholder="research-agent"
                onKeyDown={(e) => {
                  if (e.key === 'Enter') handleCreate()
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
                createAgentMutation.isPending ||
                rolesLoading ||
                (!isCustomRole && !selectedRole)
              }
              className="w-full"
            >
              {createAgentMutation.isPending ? (
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
          {providers.map((p) => (
            <SelectItem key={p.id} value={p.id}>
              {p.name} — {p.modelId}
              {p.id === defaultProviderId ? ' (default)' : ''}
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
