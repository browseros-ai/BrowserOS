import { Loader2 } from 'lucide-react'
import { type FC, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'
import { getAgentServerUrl } from '@/lib/browseros/helpers'
import { useLlmProviders } from '@/lib/llm-providers/useLlmProviders'
import { AgentList } from './AgentList'
import { AgentsHeader } from './AgentsHeader'
import { AgentTerminal } from './AgentTerminal'
import type { HarnessAgent, HarnessAgentAdapter } from './agent-harness-types'
import { buildAgentApiUrl } from './agent-api-url'
import { createAgentPageActions } from './agents-page-actions'
import {
  useDefaultAgentName,
  useHarnessAgentDefaults,
  useHermesProviderSelection,
  useOpenClawProviderSelection,
} from './agents-page-hooks'
import {
  type CreateAgentRuntime,
  DEFAULT_CREATE_RUNTIME,
  DEFAULT_HARNESS_ADAPTER,
} from './agents-page-types'
import {
  canManageOpenClawAgents,
  getAgentsLoading,
  getControlPlaneCopyForStatus,
  getGatewayUiState,
  getInlineError,
  getLifecycleBanner,
  getRecoveryDetail,
  getVisibleOpenClawAgents,
  shouldShowControlPlaneDegraded,
  toHarnessListItem,
  toOpenClawListItem,
} from './agents-page-utils'
import { GatewayStatusBar } from './GatewayStatusBar'
import { NewAgentDialog } from './NewAgentDialog'
import {
  ControlPlaneAlert,
  GatewayStateCards,
  InlineErrorAlert,
  LifecycleAlert,
} from './OpenClawControls'
import { SetupOpenClawDialog } from './SetupOpenClawDialog'
import {
  useAgentAdapters,
  useCreateHarnessAgent,
  useDeleteHarnessAgent,
  useHarnessAgents,
  useUpdateHarnessAgent,
} from './useAgents'
import { useOpenClawAgents, useOpenClawMutations } from './useOpenClaw'

export const AgentsPage: FC = () => {
  const navigate = useNavigate()
  const { providers, defaultProviderId } = useLlmProviders()
  const {
    adapters,
    loading: adaptersLoading,
    error: adaptersError,
  } = useAgentAdapters()

  // The harness listing now carries the gateway lifecycle snapshot
  // alongside the agents — one polling source for everything the
  // agents page renders. The legacy `/claw/status` poll is dead from
  // this surface; the chat-panel layout still uses it for now.
  const {
    harnessAgents,
    gateway: status,
    loading: harnessAgentsLoading,
    error: harnessAgentsError,
    refetch: refetchHarnessAgents,
  } = useHarnessAgents()

  const openClawAgentsEnabled =
    status?.status === 'running' && status.controlPlaneStatus === 'connected'
  const {
    agents: openClawAgents,
    loading: openClawAgentsLoading,
    error: openClawAgentsError,
  } = useOpenClawAgents(openClawAgentsEnabled)
  const createHarnessAgent = useCreateHarnessAgent()
  const deleteHarnessAgent = useDeleteHarnessAgent()
  const updateHarnessAgent = useUpdateHarnessAgent()
  const {
    setupOpenClaw,
    createAgent: createOpenClawAgent,
    deleteAgent: deleteOpenClawAgent,
    startOpenClaw,
    restartOpenClaw,
    reconnectOpenClaw,
    actionInProgress,
    settingUp,
    creating: creatingOpenClawAgent,
    deleting: deletingOpenClawAgent,
    reconnecting,
    pendingGatewayAction,
  } = useOpenClawMutations()

  const [setupOpen, setSetupOpen] = useState(false)
  const [setupProviderId, setSetupProviderId] = useState('')
  const [createOpen, setCreateOpen] = useState(false)
  const [newName, setNewName] = useState('')
  const [createRuntime, setCreateRuntime] = useState<CreateAgentRuntime>(
    DEFAULT_CREATE_RUNTIME,
  )
  const [createProviderId, setCreateProviderId] = useState('')
  const [harnessAdapterId, setHarnessAdapterId] = useState<HarnessAgentAdapter>(
    DEFAULT_HARNESS_ADAPTER,
  )
  const [harnessModelId, setHarnessModelId] = useState('')
  const [harnessReasoningEffort, setHarnessReasoningEffort] = useState('')
  const [createHermesProviderId, setCreateHermesProviderId] = useState('')
  const [customCommand, setCustomCommand] = useState('')
  const [customArgs, setCustomArgs] = useState('')
  const [customLabel, setCustomLabel] = useState('')
  const [customProbeResult, setCustomProbeResult] = useState<{
    healthy: boolean
    error?: string
  } | null>(null)
  const [customProbeLoading, setCustomProbeLoading] = useState(false)
  const [showTerminal, setShowTerminal] = useState(false)
  const [cliAuthModalOpen, setCliAuthModalOpen] = useState(false)
  const [pageError, setPageError] = useState<string | null>(null)
  const [createError, setCreateError] = useState<string | null>(null)
  const [deletingAgentKey, setDeletingAgentKey] = useState<string | null>(null)

  const {
    selectableOpenClawProviders,
    selectedCliProvider,
    selectedSetupCliProvider,
    authTerminalProvider,
    cliAuthStatus,
    cliAuthLoading,
    cliAuthError,
  } = useOpenClawProviderSelection({
    providers,
    defaultProviderId,
    createOpen,
    createRuntime,
    createProviderId,
    setCreateProviderId,
    setupOpen,
    setupProviderId,
    setSetupProviderId,
    cliAuthModalOpen,
    setCliAuthModalOpen,
  })
  const { selectableHermesProviders } = useHermesProviderSelection({
    providers,
    defaultProviderId,
    createOpen,
    createRuntime,
    createHermesProviderId,
    setCreateHermesProviderId,
  })
  useDefaultAgentName(createOpen, setNewName)
  useHarnessAgentDefaults({
    adapters,
    createOpen,
    harnessAdapterId,
    setHarnessAdapterId,
    setHarnessModelId,
    setHarnessReasoningEffort,
  })

  const lifecyclePending = pendingGatewayAction !== null
  const gatewayUiState = useMemo(() => getGatewayUiState(status), [status])
  const openClawManageable = canManageOpenClawAgents(
    gatewayUiState,
    lifecyclePending,
  )
  const visibleOpenClawAgents = getVisibleOpenClawAgents(
    openClawAgentsEnabled,
    openClawAgents,
  )
  const agentListItems = useMemo(() => {
    // Dual-created OpenClaw agents (and the backfilled `main`/orphans
    // post Step 9) live in both `/claw/agents` and `/agents` under the
    // same id. Prefer the harness entry — it carries adapter/model/
    // reasoning/lastUsedAt/status that the chat path actually uses —
    // and drop the legacy duplicate so the rail doesn't show every
    // OpenClaw agent twice.
    const harnessIds = new Set(harnessAgents.map((agent) => agent.id))
    const dedupedOpenClawAgents = visibleOpenClawAgents.filter(
      (agent) => !harnessIds.has(agent.agentId),
    )
    return [
      ...dedupedOpenClawAgents.map((agent) =>
        toOpenClawListItem(agent, openClawManageable),
      ),
      ...harnessAgents.map(toHarnessListItem),
    ]
  }, [harnessAgents, openClawManageable, visibleOpenClawAgents])
  // Lookup map so AgentList can render adapter chips, reasoning, etc.
  // Computed up here to keep all hooks above the early returns below.
  const harnessAgentLookup = useMemo(() => {
    const map = new Map<string, HarnessAgent>()
    for (const agent of harnessAgents) map.set(agent.id, agent)
    return map
  }, [harnessAgents])
  // Activity map keyed by agentId. Sourced from the harness listing's
  // server-side enrichment (`status` + `lastUsedAt`). Legacy gateway
  // agents that don't have a harness record yet (rare post-backfill)
  // simply miss from the map and render with the default `unknown`
  // dot until reconciliation picks them up.
  const agentActivity = useMemo(() => {
    const map: Record<
      string,
      {
        status: 'working' | 'idle' | 'asleep' | 'error'
        lastUsedAt: number | null
      }
    > = {}
    for (const agent of harnessAgents) {
      if (!agent.status) continue
      map[agent.id] = {
        status: agent.status,
        lastUsedAt: agent.lastUsedAt ?? null,
      }
    }
    return map
  }, [harnessAgents])
  const inlineError = getInlineError({
    lifecyclePending,
    pageError,
    openClawAgentsError,
    adaptersError,
    harnessAgentsError,
  })
  const agentsLoading = getAgentsLoading({
    adaptersLoading,
    harnessAgentsLoading,
    openClawAgentsLoading,
  })
  const creatingAgent = creatingOpenClawAgent || createHarnessAgent.isPending
  const deletingAgent = deletingOpenClawAgent || deleteHarnessAgent.isPending

  const handleProbeCustom = async () => {
    if (!customCommand.trim()) return
    setCustomProbeLoading(true)
    setCustomProbeResult(null)
    try {
      const baseUrl = await getAgentServerUrl()
      const response = await fetch(buildAgentApiUrl(baseUrl, '/probe-custom'), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          command: customCommand.trim(),
          args: customArgs
            .split(/\s+/)
            .map((value) => value.trim())
            .filter(Boolean),
        }),
      })
      const result = (await response.json()) as {
        healthy?: boolean
        error?: string
      }
      setCustomProbeResult({
        healthy: Boolean(result.healthy),
        ...(result.error ? { error: result.error } : {}),
      })
    } catch (err) {
      setCustomProbeResult({
        healthy: false,
        error: err instanceof Error ? err.message : 'Probe failed',
      })
    } finally {
      setCustomProbeLoading(false)
    }
  }

  const handleImportAcpx = async () => {
    try {
      const baseUrl = await getAgentServerUrl()
      const response = await fetch(buildAgentApiUrl(baseUrl, '/import-acpx'), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      })
      const result = (await response.json()) as {
        results?: Array<{ imported?: boolean; reason?: string }>
        error?: string
      }
      if (!response.ok || result.error) {
        setCreateError(result.error ?? 'Failed to import ACPX agents')
        return
      }
      const importedCount = result.results?.filter((entry) => entry.imported).length ?? 0
      if (importedCount === 0) {
        setCreateError(
          result.results?.[0]?.reason ?? 'No ACPX agents available to import',
        )
        return
      }
      setCreateError(null)
      setCreateOpen(false)
      await refetchHarnessAgents()
    } catch (err) {
      setCreateError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleHarnessAdapterChange = (adapter: HarnessAgentAdapter) => {
    const descriptor = adapters.find((entry) => entry.id === adapter)
    setHarnessAdapterId(adapter)
    setHarnessModelId(descriptor?.defaultModelId ?? '')
    setHarnessReasoningEffort(descriptor?.defaultReasoningEffort ?? '')
  }

  const { handleCreate, handleDelete, handleSetup, runWithPageErrorHandling } =
    createAgentPageActions({
      createProviderId,
      createRuntime,
      createHermesProviderId,
      customArgs,
      customCommand,
      customLabel,
      harnessModelId,
      harnessReasoningEffort,
      navigate,
      newName,
      selectableOpenClawProviders,
      selectableHermesProviders,
      setupProviderId,
      createHarnessAgent: createHarnessAgent.mutateAsync,
      createOpenClawAgent,
      deleteHarnessAgent: deleteHarnessAgent.mutateAsync,
      deleteOpenClawAgent,
      setCliAuthModalOpen,
      setCreateError,
      setCreateOpen,
      setDeletingAgentKey,
      setNewName,
      setPageError,
      setSetupOpen,
      setupOpenClaw,
    })

  if (showTerminal) {
    return <AgentTerminal onBack={() => setShowTerminal(false)} />
  }

  if (cliAuthModalOpen && authTerminalProvider) {
    return (
      <AgentTerminal
        onBack={() => setCliAuthModalOpen(false)}
        initialCommand={authTerminalProvider.authLoginCommand}
        onSessionExit={() => setCliAuthModalOpen(false)}
      />
    )
  }

  // First-paint loader: until the harness listing has resolved at
  // least once we don't know which adapters / agents to render.
  if (harnessAgentsLoading && !status) {
    return (
      <div className="flex items-center justify-center py-20">
        <Loader2 className="size-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  const showControlPlaneDegraded = shouldShowControlPlaneDegraded(
    gatewayUiState,
    lifecyclePending,
  )
  const lifecycleBanner = getLifecycleBanner(pendingGatewayAction)
  const recoveryDetail = status ? getRecoveryDetail(status) : null
  const controlPlaneCopy = getControlPlaneCopyForStatus(status)

  // Bar only makes sense when the gateway is meaningfully alive AND
  // there's at least one OpenClaw agent in the merged list. Hide it
  // for Claude/Codex-only setups so the page stays uncluttered.
  const showGatewayStatusBar =
    status?.status === 'running' &&
    (visibleOpenClawAgents.length > 0 ||
      harnessAgents.some((agent) => agent.adapter === 'openclaw'))

  return (
    <div className="min-h-full bg-background px-6 py-8">
      <div className="fade-in slide-in-from-bottom-5 mx-auto flex w-full max-w-5xl animate-in flex-col gap-6 duration-500">
        <AgentsHeader onCreateAgent={() => setCreateOpen(true)} />

        {lifecycleBanner ? <LifecycleAlert message={lifecycleBanner} /> : null}

        {inlineError ? (
          <InlineErrorAlert
            message={inlineError}
            onDismiss={() => setPageError(null)}
          />
        ) : null}

        {status && showControlPlaneDegraded ? (
          <ControlPlaneAlert
            actionInProgress={actionInProgress}
            controlPlaneBusy={gatewayUiState.controlPlaneBusy}
            controlPlaneCopy={controlPlaneCopy}
            reconnecting={reconnecting}
            recoveryDetail={recoveryDetail}
            status={status}
            onReconnect={() => {
              void runWithPageErrorHandling(reconnectOpenClaw)
            }}
            onRestart={() => {
              void runWithPageErrorHandling(restartOpenClaw)
            }}
          />
        ) : null}

        <GatewayStateCards
          actionInProgress={actionInProgress}
          status={status}
          onOpenSetup={() => setSetupOpen(true)}
          onRestart={() => {
            void runWithPageErrorHandling(restartOpenClaw)
          }}
          onStart={() => {
            void runWithPageErrorHandling(startOpenClaw)
          }}
        />

        {showGatewayStatusBar ? (
          <GatewayStatusBar
            status={status}
            actionInProgress={actionInProgress}
            onOpenTerminal={() => setShowTerminal(true)}
            onRestart={() => {
              void runWithPageErrorHandling(restartOpenClaw)
            }}
          />
        ) : null}

        <AgentList
          agents={agentListItems}
          activity={agentActivity}
          harnessAgentLookup={harnessAgentLookup}
          adapters={adapters}
          loading={agentsLoading}
          deletingAgentKey={deletingAgent ? deletingAgentKey : null}
          onCreateAgent={() => setCreateOpen(true)}
          onDeleteAgent={(agent) => {
            void handleDelete(agent)
          }}
          onPinToggle={(agent, next) => {
            // Optimistic mutation; harness-only — gateway-original
            // OpenClaw entries are gated server-side via the harness
            // backfill, so we only fire when the row maps to a
            // harness agent record.
            if (!harnessAgentLookup.has(agent.agentId)) return
            updateHarnessAgent.mutate({
              agentId: agent.agentId,
              patch: { pinned: next },
            })
          }}
        />

        <SetupOpenClawDialog
          defaultProviderId={defaultProviderId}
          open={setupOpen}
          providers={selectableOpenClawProviders}
          selectedProviderId={setupProviderId}
          selectedCliProvider={selectedSetupCliProvider}
          settingUp={settingUp}
          onOpenChange={setSetupOpen}
          onProviderChange={setSetupProviderId}
          onSetup={() => void handleSetup()}
        />

        <NewAgentDialog
          adapters={adapters}
          canManageOpenClaw={openClawManageable}
          createError={createError}
          createRuntime={createRuntime}
          creating={creatingAgent}
          defaultProviderId={defaultProviderId}
          harnessAdapterId={harnessAdapterId}
          harnessModelId={harnessModelId}
          harnessReasoningEffort={harnessReasoningEffort}
          hermesProviders={selectableHermesProviders}
          hermesSelectedProviderId={createHermesProviderId}
          name={newName}
          open={createOpen}
          providers={selectableOpenClawProviders}
          selectedCliProvider={selectedCliProvider}
          selectedProviderId={createProviderId}
          cliAuthError={cliAuthError ?? null}
          cliAuthLoading={cliAuthLoading}
          cliAuthStatus={cliAuthStatus}
          customCommand={customCommand}
          customArgs={customArgs}
          customLabel={customLabel}
          customProbeResult={customProbeResult}
          customProbeLoading={customProbeLoading}
          onConnectCliProvider={() => setCliAuthModalOpen(true)}
          onCreate={handleCreate}
          onOpenChange={(open) => {
            setCreateOpen(open)
            if (!open) {
              setCreateError(null)
              createHarnessAgent.reset()
              setCreateHermesProviderId('')
              setCustomCommand('')
              setCustomArgs('')
              setCustomLabel('')
              setCustomProbeResult(null)
              setCustomProbeLoading(false)
            }
          }}
          onRuntimeChange={setCreateRuntime}
          onHarnessAdapterChange={handleHarnessAdapterChange}
          onHarnessModelChange={setHarnessModelId}
          onHarnessReasoningChange={setHarnessReasoningEffort}
          onHermesProviderChange={setCreateHermesProviderId}
          onNameChange={setNewName}
          onProviderChange={setCreateProviderId}
          onCustomCommandChange={setCustomCommand}
          onCustomArgsChange={setCustomArgs}
          onCustomLabelChange={setCustomLabel}
          onProbeCustom={handleProbeCustom}
          onImportAcpx={handleImportAcpx}
        />
      </div>
    </div>
  )
}
