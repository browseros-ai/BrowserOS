import { Loader2 } from 'lucide-react'
import { type FC, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'
import { useLlmProviders } from '@/lib/llm-providers/useLlmProviders'
import { AgentList } from './AgentList'
import { AgentTerminal } from './AgentTerminal'
import type { HarnessAgentAdapter } from './agent-harness-types'
import { createAgentPageActions } from './agents-page-actions'
import {
  useDefaultAgentName,
  useHarnessAgentDefaults,
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
import { NewAgentDialog } from './NewAgentDialog'
import {
  AgentsPageHeader,
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
} from './useAgents'
import {
  useOpenClawAgents,
  useOpenClawMutations,
  useOpenClawStatus,
} from './useOpenClaw'

export const AgentsPage: FC = () => {
  const navigate = useNavigate()
  const {
    status,
    loading: statusLoading,
    error: statusError,
    refetch: refetchStatus,
  } = useOpenClawStatus()
  const { providers, defaultProviderId } = useLlmProviders()
  const {
    adapters,
    loading: adaptersLoading,
    error: adaptersError,
    refetch: refetchAdapters,
  } = useAgentAdapters()

  const openClawAgentsEnabled =
    status?.status === 'running' && status.controlPlaneStatus === 'connected'
  const {
    agents: openClawAgents,
    loading: openClawAgentsLoading,
    error: openClawAgentsError,
    refetch: refetchOpenClawAgents,
  } = useOpenClawAgents(openClawAgentsEnabled)
  const {
    harnessAgents,
    loading: harnessAgentsLoading,
    error: harnessAgentsError,
    refetch: refetchHarnessAgents,
  } = useHarnessAgents()
  const createHarnessAgent = useCreateHarnessAgent()
  const deleteHarnessAgent = useDeleteHarnessAgent()
  const {
    setupOpenClaw,
    createAgent: createOpenClawAgent,
    deleteAgent: deleteOpenClawAgent,
    startOpenClaw,
    stopOpenClaw,
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
  const agentListItems = useMemo(
    () => [
      ...visibleOpenClawAgents.map((agent) =>
        toOpenClawListItem(agent, openClawManageable),
      ),
      ...harnessAgents.map(toHarnessListItem),
    ],
    [harnessAgents, openClawManageable, visibleOpenClawAgents],
  )
  const inlineError = getInlineError({
    lifecyclePending,
    pageError,
    statusError,
    openClawAgentsError,
    adaptersError,
    harnessAgentsError,
  })
  const agentsLoading = getAgentsLoading({
    statusLoading,
    adaptersLoading,
    harnessAgentsLoading,
    openClawAgentsEnabled,
    openClawAgentsLoading,
  })
  const creatingAgent = creatingOpenClawAgent || createHarnessAgent.isPending
  const deletingAgent = deletingOpenClawAgent || deleteHarnessAgent.isPending

  const refreshAll = async () => {
    await Promise.all([
      refetchStatus(),
      refetchAdapters(),
      refetchHarnessAgents(),
      openClawAgentsEnabled ? refetchOpenClawAgents() : Promise.resolve(),
    ])
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
      harnessModelId,
      harnessReasoningEffort,
      navigate,
      newName,
      selectableOpenClawProviders,
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

  if (statusLoading && !status) {
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

  return (
    <div className="min-h-full bg-background px-6 py-8">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
        <AgentsPageHeader
          actionInProgress={actionInProgress}
          controlPlaneBusy={gatewayUiState.controlPlaneBusy}
          reconnecting={reconnecting}
          status={status}
          onCreateAgent={() => setCreateOpen(true)}
          onOpenTerminal={() => setShowTerminal(true)}
          onReconnect={() => {
            void runWithPageErrorHandling(reconnectOpenClaw)
          }}
          onRefresh={() => void refreshAll()}
          onRestart={() => {
            void runWithPageErrorHandling(restartOpenClaw)
          }}
          onStop={() => {
            void runWithPageErrorHandling(stopOpenClaw)
          }}
        />

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

        <AgentList
          agents={agentListItems}
          loading={agentsLoading}
          deletingAgentKey={deletingAgent ? deletingAgentKey : null}
          onChatAgent={(agent) => navigate(`/agents/${agent.agentId}`)}
          onCreateAgent={() => setCreateOpen(true)}
          onDeleteAgent={(agent) => {
            void handleDelete(agent)
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
          name={newName}
          open={createOpen}
          providers={selectableOpenClawProviders}
          selectedCliProvider={selectedCliProvider}
          selectedProviderId={createProviderId}
          cliAuthError={cliAuthError ?? null}
          cliAuthLoading={cliAuthLoading}
          cliAuthStatus={cliAuthStatus}
          onConnectCliProvider={() => setCliAuthModalOpen(true)}
          onCreate={handleCreate}
          onOpenChange={(open) => {
            setCreateOpen(open)
            if (!open) {
              setCreateError(null)
              createHarnessAgent.reset()
            }
          }}
          onRuntimeChange={setCreateRuntime}
          onHarnessAdapterChange={handleHarnessAdapterChange}
          onHarnessModelChange={setHarnessModelId}
          onHarnessReasoningChange={setHarnessReasoningEffort}
          onNameChange={setNewName}
          onProviderChange={setCreateProviderId}
        />
      </div>
    </div>
  )
}
