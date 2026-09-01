import { useQueryClient } from '@tanstack/react-query'
import { Plus } from 'lucide-react'
import { type FC, useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { BrowserClawPromoBanner } from '@/components/promo/BrowserClawPromoBanner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { GetProfileIdByUserIdDocument } from '@/lib/conversations/graphql/uploadConversationDocument'
import { getQueryKeyFromDocument } from '@/lib/graphql/getQueryKeyFromDocument'
import { testProvider } from '@/lib/llm-providers/testProvider'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { track } from '@/lib/metrics/track'
import { sentry } from '@/lib/sentry/sentry'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import { useGraphqlMutation } from '@/modules/graphql/graphql-mutation.hooks'
import { useGraphqlQuery } from '@/modules/graphql/graphql-query.hooks'
import { useLlmProviders } from '@/modules/llm-providers/llm-providers.hooks'
import { AddProviderSection } from './AddProviderSection'
import { AddProviderDialogs, useAddProvider } from './add-provider.hooks'
import { ConfiguredTargetsList } from './ConfiguredTargetsList'
import { useCodingAgents } from './coding-agents.hooks'
import { useDefaultChatTarget } from './default-chat-target.hooks'
import {
  DeleteRemoteLlmProviderDocument,
  GetRemoteLlmProvidersDocument,
} from './graphql/aiSettingsDocument'
import type { IncompleteProvider } from './IncompleteProviderCard'
import { IncompleteProvidersList } from './IncompleteProvidersList'
import { McpPromoBanner } from './McpPromoBanner'
import { NewProviderDialog } from './NewProviderDialog'
import { partitionSyncedProviders } from './synced-providers'

/**
 * BrowserOS AI pane — manage LLM providers and the default model.
 */
export const BrowserOsAiPane: FC = () => {
  const {
    providers,
    defaultProviderId,
    saveProvider,
    setDefaultProvider,
    deleteProvider,
  } = useLlmProviders()
  const { baseUrl: agentServerUrl } = useAgentServerUrl()
  const { sessionInfo } = useSessionInfo()
  const queryClient = useQueryClient()
  const coding = useCodingAgents()
  const defaultTarget = useDefaultChatTarget({
    providers,
    agents: coding.agents,
    defaultProviderId,
    setDefaultProvider,
  })
  const { effectiveTarget } = defaultTarget
  const selectedProviderId =
    effectiveTarget.kind === 'llm' ? effectiveTarget.id : null
  const selectedAgentId =
    effectiveTarget.kind === 'acp' ? effectiveTarget.id : null

  const userId = sessionInfo.user?.id

  const { data: profileData } = useGraphqlQuery(
    GetProfileIdByUserIdDocument,
    // biome-ignore lint/style/noNonNullAssertion: guarded by enabled
    { userId: userId! },
    { enabled: !!userId },
  )
  const profileId = profileData?.profileByUserId?.rowId

  const { data: remoteProvidersData } = useGraphqlQuery(
    GetRemoteLlmProvidersDocument,
    // biome-ignore lint/style/noNonNullAssertion: guarded by enabled
    { profileId: profileId! },
    { enabled: !!profileId },
  )

  const { mutate: deleteRemoteProvider } = useGraphqlMutation(
    DeleteRemoteLlmProviderDocument,
    {
      onSuccess: () => {
        queryClient.invalidateQueries({
          queryKey: [getQueryKeyFromDocument(GetRemoteLlmProvidersDocument)],
        })
      },
      onError: (error, { rowId }) => {
        sentry.captureException(error, {
          extra: {
            message: 'Failed to delete a synced provider',
            providerId: rowId,
          },
        })
      },
    },
  )

  const { incompleteProviders, retiredProviderIds } = useMemo(() => {
    if (!remoteProvidersData?.llmProviders?.nodes) {
      return { incompleteProviders: [], retiredProviderIds: [] }
    }
    const localProviderIds = new Set(providers.map((p) => p.id))
    return partitionSyncedProviders(
      remoteProvidersData.llmProviders.nodes,
      localProviderIds,
    )
  }, [remoteProvidersData, providers])

  useEffect(() => {
    for (const rowId of retiredProviderIds) {
      deleteRemoteProvider({ rowId })
    }
  }, [deleteRemoteProvider, retiredProviderIds])

  const [isEditDialogOpen, setIsEditDialogOpen] = useState(false)
  const [editingProvider, setEditingProvider] =
    useState<LlmProviderConfig | null>(null)
  const [providerToDelete, setProviderToDelete] =
    useState<LlmProviderConfig | null>(null)
  const [incompleteProviderToDelete, setIncompleteProviderToDelete] =
    useState<IncompleteProvider | null>(null)
  const [testingProviderId, setTestingProviderId] = useState<string | null>(
    null,
  )

  const addProvider = useAddProvider({ providers, saveProvider })
  const { oauthFlows } = addProvider

  const handleEditProvider = (provider: LlmProviderConfig) => {
    setEditingProvider(provider)
    setIsEditDialogOpen(true)
  }

  const handleDeleteProvider = (provider: LlmProviderConfig) => {
    setProviderToDelete(provider)
  }

  const confirmDeleteProvider = async () => {
    if (!providerToDelete) return

    // Clear OAuth tokens on server for OAuth-based providers
    const oauthFlow = oauthFlows[providerToDelete.type]
    if (oauthFlow) {
      await oauthFlow.disconnect()
      track(oauthFlow.disconnectedEvent)
    }

    await deleteProvider(providerToDelete.id)
    deleteRemoteProvider({ rowId: providerToDelete.id })

    setProviderToDelete(null)
  }

  const handleAddKeysToIncomplete = (provider: IncompleteProvider) => {
    const timestamp = Date.now()
    addProvider.openProviderForm({
      id: provider.rowId,
      type: provider.type as LlmProviderConfig['type'],
      name: provider.name,
      baseUrl: provider.baseUrl ?? undefined,
      modelId: provider.modelId,
      supportsImages: provider.supportsImages,
      contextWindow: provider.contextWindow ?? 128000,
      temperature: provider.temperature ?? 0.2,
      resourceName: provider.resourceName ?? undefined,
      region: provider.region ?? undefined,
      createdAt: timestamp,
      updatedAt: timestamp,
    })
  }

  const handleDeleteIncompleteProvider = (provider: IncompleteProvider) => {
    setIncompleteProviderToDelete(provider)
  }

  const confirmDeleteIncompleteProvider = () => {
    if (incompleteProviderToDelete) {
      deleteRemoteProvider({
        rowId: incompleteProviderToDelete.rowId,
      })
      setIncompleteProviderToDelete(null)
    }
  }

  const handleSaveProvider = async (provider: LlmProviderConfig) => {
    await saveProvider(provider)
  }

  const handleTestProvider = async (provider: LlmProviderConfig) => {
    if (!agentServerUrl) {
      toast.error('Test Failed', {
        description: (
          <span className="text-red-600 text-sm dark:text-red-400">
            Server URL not available
          </span>
        ),
        duration: 3000,
      })
      return
    }

    setTestingProviderId(provider.id)

    try {
      const result = await testProvider(provider, agentServerUrl)

      if (result.success) {
        toast.success('Test Successful', {
          description: (
            <span className="text-green-600 text-sm dark:text-green-400">
              {result.message}
            </span>
          ),
          duration: 3000,
        })
      } else {
        toast.error('Test Failed', {
          description: (
            <span className="text-red-600 text-sm dark:text-red-400">
              {result.message}
            </span>
          ),
          duration: 3000,
        })
      }
    } catch (error) {
      toast.error('Test Failed', {
        description: (
          <span className="text-red-600 text-sm dark:text-red-400">
            {error instanceof Error ? error.message : 'Unknown error'}
          </span>
        ),
        duration: 3000,
      })
    }

    setTestingProviderId(null)
  }

  return (
    <div className="fade-in slide-in-from-bottom-5 animate-in space-y-6 duration-500">
      <div>
        <h2 className="font-semibold text-xl">AI &amp; Agents</h2>
        <p className="text-muted-foreground text-sm">
          Pick what runs your chats, and connect anything else you use.
        </p>
      </div>

      <BrowserClawPromoBanner />

      <section className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <h3 className="font-semibold text-base">
            Your providers{' '}
            <span className="font-normal text-muted-foreground">
              ({providers.length + coding.agents.length})
            </span>
          </h3>
          <Button onClick={() => addProvider.openProviderForm()}>
            <Plus className="size-4" />
            Add
          </Button>
        </div>

        <ConfiguredTargetsList
          providers={providers}
          coding={coding}
          selectedProviderId={selectedProviderId}
          selectedAgentId={selectedAgentId}
          testingProviderId={testingProviderId}
          onSelectProvider={defaultTarget.selectProvider}
          onSelectAgent={defaultTarget.selectAgent}
          onTestProvider={handleTestProvider}
          onEditProvider={handleEditProvider}
          onDeleteProvider={handleDeleteProvider}
          onEditAgent={addProvider.openCustomAgentEditor}
        />
      </section>

      <AddProviderSection
        onCreateAgent={addProvider.onCreateAgent}
        onCreateCustomAgent={addProvider.onCreateCustomAgent}
        onUseTemplate={addProvider.onUseTemplate}
      />

      <McpPromoBanner />

      <IncompleteProvidersList
        providers={incompleteProviders}
        onAddKeys={handleAddKeysToIncomplete}
        onDelete={handleDeleteIncompleteProvider}
      />

      <AddProviderDialogs controller={addProvider} />

      <NewProviderDialog
        open={isEditDialogOpen}
        onOpenChange={setIsEditDialogOpen}
        initialValues={editingProvider ?? undefined}
        onSave={handleSaveProvider}
      />

      <AlertDialog
        open={!!providerToDelete}
        onOpenChange={(open) => !open && setProviderToDelete(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Provider</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete "{providerToDelete?.name}"? This
              action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={confirmDeleteProvider}>
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={!!incompleteProviderToDelete}
        onOpenChange={(open) => !open && setIncompleteProviderToDelete(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Synced Provider</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete "
              {incompleteProviderToDelete?.name}
              "? This will remove it from all your devices.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={confirmDeleteIncompleteProvider}>
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
