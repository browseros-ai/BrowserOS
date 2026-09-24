import { Plus } from 'lucide-react'
import { type FC, useState } from 'react'
import { toast } from 'sonner'
import { CloudSyncRetiredNotice } from '@/components/cloud-sync/CloudSyncRetiredNotice'
import { BrowserClawPromoBanner } from '@/components/promo/BrowserClawPromoBanner'
import { Alert, AlertDescription } from '@/components/ui/alert'
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
import { testProvider } from '@/lib/llm-providers/testProvider'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { track } from '@/lib/metrics/track'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import { useLlmProviders } from '@/modules/llm-providers/llm-providers.hooks'
import { AddProviderSection } from './AddProviderSection'
import { AddProviderDialogs, useAddProvider } from './add-provider.hooks'
import { ConfiguredTargetsList } from './ConfiguredTargetsList'
import { useCodingAgents } from './coding-agents.hooks'
import { useDefaultChatTarget } from './default-chat-target.hooks'
import { NewProviderDialog } from './NewProviderDialog'

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
    isUnavailable: providersUnavailable,
  } = useLlmProviders()
  const { baseUrl: agentServerUrl } = useAgentServerUrl()
  const coding = useCodingAgents()
  const defaultTarget = useDefaultChatTarget({
    providers,
    agents: coding.agents,
    defaultProviderId,
    setDefaultProvider,
  })
  const { effectiveTarget } = defaultTarget
  const selectedProviderId =
    effectiveTarget?.kind === 'llm' ? effectiveTarget.id : null
  const selectedAgentId =
    effectiveTarget?.kind === 'acp' ? effectiveTarget.id : null

  const [isEditDialogOpen, setIsEditDialogOpen] = useState(false)
  const [editingProvider, setEditingProvider] =
    useState<LlmProviderConfig | null>(null)
  const [providerToDelete, setProviderToDelete] =
    useState<LlmProviderConfig | null>(null)
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
    setProviderToDelete(null)
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

      <CloudSyncRetiredNotice />

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

        {providersUnavailable ? (
          <Alert variant="destructive">
            <AlertDescription>
              Your providers could not be loaded because the BrowserOS server is
              not reachable. They are still saved on this device.
            </AlertDescription>
          </Alert>
        ) : null}

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
    </div>
  )
}
