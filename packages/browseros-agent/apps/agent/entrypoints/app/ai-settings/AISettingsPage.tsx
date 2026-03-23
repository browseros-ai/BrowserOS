import { useQueryClient } from '@tanstack/react-query'
import { type FC, useMemo, useState } from 'react'
import { toast } from 'sonner'
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
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import {
  CHATGPT_PRO_OAUTH_COMPLETED_EVENT,
  CHATGPT_PRO_OAUTH_DISCONNECTED_EVENT,
  CHATGPT_PRO_OAUTH_STARTED_EVENT,
  GITHUB_COPILOT_OAUTH_COMPLETED_EVENT,
  GITHUB_COPILOT_OAUTH_DISCONNECTED_EVENT,
  GITHUB_COPILOT_OAUTH_STARTED_EVENT,
  QWEN_CODE_OAUTH_COMPLETED_EVENT,
  QWEN_CODE_OAUTH_DISCONNECTED_EVENT,
  QWEN_CODE_OAUTH_STARTED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { GetProfileIdByUserIdDocument } from '@/lib/conversations/graphql/uploadConversationDocument'
import { getQueryKeyFromDocument } from '@/lib/graphql/getQueryKeyFromDocument'
import { useGraphqlMutation } from '@/lib/graphql/useGraphqlMutation'
import { useGraphqlQuery } from '@/lib/graphql/useGraphqlQuery'
import type { ProviderTemplate } from '@/lib/llm-providers/providerTemplates'
import { testProvider } from '@/lib/llm-providers/testProvider'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { useLlmProviders } from '@/lib/llm-providers/useLlmProviders'
import {
  type OAuthProviderFlowConfig,
  useOAuthProviderFlow,
} from '@/lib/llm-providers/useOAuthProviderFlow'
import { track } from '@/lib/metrics/track'
import { ConfiguredProvidersList } from './ConfiguredProvidersList'
import {
  DeleteRemoteLlmProviderDocument,
  GetRemoteLlmProvidersDocument,
} from './graphql/aiSettingsDocument'
import type { IncompleteProvider } from './IncompleteProviderCard'
import { IncompleteProvidersList } from './IncompleteProvidersList'
import { LlmProvidersHeader } from './LlmProvidersHeader'
import { McpPromoBanner } from './McpPromoBanner'
import { NewProviderDialog } from './NewProviderDialog'
import { ProviderTemplatesSection } from './ProviderTemplatesSection'

// All OAuth providers share the same flow via useOAuthProviderFlow
const OAUTH_PROVIDERS_CONFIG: Record<string, OAuthProviderFlowConfig> = {
  'chatgpt-pro': {
    providerType: 'chatgpt-pro',
    displayName: 'ChatGPT Plus/Pro',
    startedEvent: CHATGPT_PRO_OAUTH_STARTED_EVENT,
    completedEvent: CHATGPT_PRO_OAUTH_COMPLETED_EVENT,
    disconnectedEvent: CHATGPT_PRO_OAUTH_DISCONNECTED_EVENT,
  },
  'github-copilot': {
    providerType: 'github-copilot',
    displayName: 'GitHub Copilot',
    startedEvent: GITHUB_COPILOT_OAUTH_STARTED_EVENT,
    completedEvent: GITHUB_COPILOT_OAUTH_COMPLETED_EVENT,
    disconnectedEvent: GITHUB_COPILOT_OAUTH_DISCONNECTED_EVENT,
    clientAuth: {
      deviceCodeEndpoint: 'https://github.com/login/device/code',
      tokenEndpoint: 'https://github.com/login/oauth/access_token',
      clientId: 'Ov23li8tweQw6odWQebz',
      scopes: 'read:user',
      requiresPKCE: false,
      contentType: 'json',
    },
  },
  'qwen-code': {
    providerType: 'qwen-code',
    displayName: 'Qwen Code',
    startedEvent: QWEN_CODE_OAUTH_STARTED_EVENT,
    completedEvent: QWEN_CODE_OAUTH_COMPLETED_EVENT,
    disconnectedEvent: QWEN_CODE_OAUTH_DISCONNECTED_EVENT,
    clientAuth: {
      deviceCodeEndpoint: 'https://chat.qwen.ai/api/v1/oauth2/device/code',
      tokenEndpoint: 'https://chat.qwen.ai/api/v1/oauth2/token',
      clientId: 'f0304373b74a44d2b584a3fb70ca9e56',
      scopes: 'openid profile email model.completion',
      requiresPKCE: true,
      contentType: 'form',
    },
  },
}

/**
 * AI Settings page for managing LLM providers
 * @public
 */
export const AISettingsPage: FC = () => {
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

  const deleteRemoteProviderMutation = useGraphqlMutation(
    DeleteRemoteLlmProviderDocument,
    {
      onSuccess: () => {
        queryClient.invalidateQueries({
          queryKey: [getQueryKeyFromDocument(GetRemoteLlmProvidersDocument)],
        })
      },
    },
  )

  const incompleteProviders = useMemo<IncompleteProvider[]>(() => {
    if (!remoteProvidersData?.llmProviders?.nodes) return []
    const localProviderIds = new Set(providers.map((p) => p.id))
    return remoteProvidersData.llmProviders.nodes
      .filter((node): node is NonNullable<typeof node> => node !== null)
      .filter((node) => !localProviderIds.has(node.rowId))
  }, [remoteProvidersData, providers])

  const [isNewDialogOpen, setIsNewDialogOpen] = useState(false)
  const [isEditDialogOpen, setIsEditDialogOpen] = useState(false)
  const [templateValues, setTemplateValues] = useState<
    Partial<LlmProviderConfig> | undefined
  >()
  const [editingProvider, setEditingProvider] =
    useState<LlmProviderConfig | null>(null)
  const [providerToDelete, setProviderToDelete] =
    useState<LlmProviderConfig | null>(null)
  const [incompleteProviderToDelete, setIncompleteProviderToDelete] =
    useState<IncompleteProvider | null>(null)
  const [testingProviderId, setTestingProviderId] = useState<string | null>(
    null,
  )

  // OAuth flows — shared hook eliminates per-provider duplication
  const chatgptPro = useOAuthProviderFlow(
    OAUTH_PROVIDERS_CONFIG['chatgpt-pro'],
    providers,
    saveProvider,
  )
  const copilot = useOAuthProviderFlow(
    OAUTH_PROVIDERS_CONFIG['github-copilot'],
    providers,
    saveProvider,
  )
  const qwenCode = useOAuthProviderFlow(
    OAUTH_PROVIDERS_CONFIG['qwen-code'],
    providers,
    saveProvider,
  )

  const oauthFlows: Record<
    string,
    {
      startOAuthFlow: (url: string | undefined) => Promise<void>
      disconnect: () => Promise<void>
      disconnectedEvent: string
    }
  > = {
    'chatgpt-pro': {
      startOAuthFlow: chatgptPro.startOAuthFlow,
      disconnect: chatgptPro.disconnect,
      disconnectedEvent: CHATGPT_PRO_OAUTH_DISCONNECTED_EVENT,
    },
    'github-copilot': {
      startOAuthFlow: copilot.startOAuthFlow,
      disconnect: copilot.disconnect,
      disconnectedEvent: GITHUB_COPILOT_OAUTH_DISCONNECTED_EVENT,
    },
    'qwen-code': {
      startOAuthFlow: qwenCode.startOAuthFlow,
      disconnect: qwenCode.disconnect,
      disconnectedEvent: QWEN_CODE_OAUTH_DISCONNECTED_EVENT,
    },
  }

  const handleAddProvider = () => {
    setTemplateValues(undefined)
    setIsNewDialogOpen(true)
  }

  const handleUseTemplate = (template: ProviderTemplate) => {
    // OAuth providers: trigger OAuth flow
    const oauthFlow = oauthFlows[template.id]
    if (oauthFlow) {
      oauthFlow.startOAuthFlow(agentServerUrl ?? undefined)
      return
    }

    setTemplateValues({
      type: template.id,
      name: template.name,
      baseUrl: template.defaultBaseUrl,
      modelId: template.defaultModelId,
      supportsImages: template.supportsImages,
      contextWindow: template.contextWindow,
      temperature: 0.2,
    })
    setIsNewDialogOpen(true)
  }

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
    deleteRemoteProviderMutation.mutate({ rowId: providerToDelete.id })
    setProviderToDelete(null)
  }

  const handleAddKeysToIncomplete = (provider: IncompleteProvider) => {
    const timestamp = Date.now()
    setTemplateValues({
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
    setIsNewDialogOpen(true)
  }

  const handleDeleteIncompleteProvider = (provider: IncompleteProvider) => {
    setIncompleteProviderToDelete(provider)
  }

  const confirmDeleteIncompleteProvider = () => {
    if (incompleteProviderToDelete) {
      deleteRemoteProviderMutation.mutate({
        rowId: incompleteProviderToDelete.rowId,
      })
      setIncompleteProviderToDelete(null)
    }
  }

  const handleSaveProvider = async (provider: LlmProviderConfig) => {
    await saveProvider(provider)
  }

  const handleSelectProvider = (providerId: string) => {
    setDefaultProvider(providerId)
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
      <LlmProvidersHeader
        providers={providers}
        defaultProviderId={defaultProviderId}
        onDefaultProviderChange={setDefaultProvider}
        onAddProvider={handleAddProvider}
      />

      <McpPromoBanner />

      <ProviderTemplatesSection onUseTemplate={handleUseTemplate} />

      <ConfiguredProvidersList
        providers={providers}
        selectedProviderId={defaultProviderId}
        testingProviderId={testingProviderId}
        onSelectProvider={handleSelectProvider}
        onTestProvider={handleTestProvider}
        onEditProvider={handleEditProvider}
        onDeleteProvider={handleDeleteProvider}
      />

      <IncompleteProvidersList
        providers={incompleteProviders}
        onAddKeys={handleAddKeysToIncomplete}
        onDelete={handleDeleteIncompleteProvider}
      />

      <NewProviderDialog
        open={isNewDialogOpen}
        onOpenChange={setIsNewDialogOpen}
        initialValues={templateValues}
        onSave={handleSaveProvider}
      />

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
