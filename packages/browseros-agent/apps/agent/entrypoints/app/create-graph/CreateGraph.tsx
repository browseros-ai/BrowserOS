import { useChat } from '@ai-sdk/react'
import { DefaultChatTransport, type UIMessage } from 'ai'
import { compact } from 'es-toolkit/array'
import type { FC, FormEvent } from 'react'
import { useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import useDeepCompareEffect from 'use-deep-compare-effect'
import type { Provider } from '@/components/chat/chatComponentTypes'
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
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from '@/components/ui/resizable'
import { useChatRefs } from '@/entrypoints/sidepanel/index/useChatRefs'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import {
  GRAPH_SAVED_EVENT,
  GRAPH_UPDATED_EVENT,
  NEW_GRAPH_CREATED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { useLlmProviders } from '@/lib/llm-providers/useLlmProviders'
import { track } from '@/lib/metrics/track'
import { useRpcClient } from '@/lib/rpc/RpcClientProvider'
import { sentry } from '@/lib/sentry/sentry'
import { useWorkflows } from '@/lib/workflows/workflowStorage'
import { GraphCanvas } from './GraphCanvas'
import { GraphChat } from './GraphChat'
import { WorkflowsChatHeader } from './WorkflowsChatHeader'

type MessageType = 'create-graph' | 'update-graph' | 'run-graph'

type GraphMessageMetadata = {
  messageType?: MessageType
  codeId?: string
  graph?: GraphData
  window?: chrome.windows.Window
}

export type GraphData = {
  nodes: {
    id: string
    type: string
    data: {
      label: string
    }
  }[]
  edges: {
    id: string
    source: string
    target: string
  }[]
}

const getLastMessageText = (messages: UIMessage[]) => {
  const lastMessage = messages[messages.length - 1]
  if (!lastMessage) return ''
  return lastMessage.parts
    .filter((part) => part.type === 'text')
    .map((part) => part.text)
    .join('')
}

export const CreateGraph: FC = () => {
  const [searchParams] = useSearchParams()
  const workflowIdParam = searchParams.get('workflowId')

  const [graphName, setGraphName] = useState('')
  const [codeId, setCodeId] = useState<string | undefined>(undefined)
  const [graphData, setGraphData] = useState<GraphData | undefined>(undefined)
  const [savedWorkflowId, setSavedWorkflowId] = useState<string | undefined>(
    undefined,
  )
  const [savedCodeId, setSavedCodeId] = useState<string | undefined>(undefined)
  const [isInitialized, setIsInitialized] = useState(!workflowIdParam)
  const [canvasPanelSize, setCanvasPanelSize] = useState<
    { asPercentage: number; inPixels: number } | undefined
  >(undefined)

  const [query, setQuery] = useState('')
  const [showDiscardDialog, setShowDiscardDialog] = useState(false)

  const { workflows, addWorkflow, editWorkflow } = useWorkflows()
  const { providers: llmProviders, setDefaultProvider } = useLlmProviders()
  const rpcClient = useRpcClient()

  // Initialize edit mode when workflowId is provided
  useDeepCompareEffect(() => {
    if (!workflowIdParam || isInitialized) return

    const workflow = workflows.find((w) => w.id === workflowIdParam)
    if (!workflow) return

    const initializeEditMode = async () => {
      setGraphName(workflow.workflowName)
      setCodeId(workflow.codeId)
      setSavedWorkflowId(workflow.id)
      setSavedCodeId(workflow.codeId)

      try {
        const response = await rpcClient.graph[':id'].$get({
          param: { id: workflow.codeId },
        })

        if (response.ok) {
          const data = await response.json()
          if ('graph' in data && data.graph) {
            setGraphData(data.graph as GraphData)
          }
        }
      } catch (error) {
        sentry.captureException(error, {
          extra: {
            message: 'Failed to fetch graph data from the server',
            codeId: workflow.codeId,
          },
        })
      }

      setIsInitialized(true)
    }

    initializeEditMode()
  }, [workflowIdParam, workflows, isInitialized, rpcClient])

  const updateQuery = (newQuery: string) => {
    setQuery(newQuery)
  }

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (codeId) {
      sendMessage({
        text: query,
        metadata: {
          messageType: 'update-graph' as MessageType,
          codeId,
        },
      })
      track(GRAPH_UPDATED_EVENT)
    } else {
      sendMessage({
        text: query,
        metadata: {
          messageType: 'create-graph' as MessageType,
        },
      })
      track(NEW_GRAPH_CREATED_EVENT)
    }
    setQuery('')
  }

  const {
    baseUrl: agentServerUrl,
    isLoading: _isLoadingAgentUrl,
    error: agentUrlError,
  } = useAgentServerUrl()

  const {
    selectedLlmProviderRef,
    enabledMcpServersRef,
    enabledCustomServersRef,
    personalizationRef,
    selectedLlmProvider,
    isLoadingProviders,
  } = useChatRefs()

  const agentUrlRef = useRef(agentServerUrl)
  const codeIdRef = useRef(codeId)

  useEffect(() => {
    agentUrlRef.current = agentServerUrl
    codeIdRef.current = codeId
  }, [agentServerUrl, codeId])

  const { sendMessage, stop, status, messages, error, setMessages } = useChat({
    transport: new DefaultChatTransport({
      prepareSendMessagesRequest: async ({ messages }) => {
        const lastMessage = messages[messages.length - 1]
        const lastMessageText = getLastMessageText(messages)
        const metadata = lastMessage.metadata as
          | GraphMessageMetadata
          | undefined

        if (metadata?.messageType === 'create-graph') {
          return {
            api: `${agentUrlRef.current}/graph`,
            body: {
              query: lastMessageText,
            },
          }
        }

        if (metadata?.messageType === 'update-graph' && codeIdRef.current) {
          return {
            api: `${agentUrlRef.current}/graph/${codeIdRef.current}`,
            body: {
              query: lastMessageText,
            },
          }
        }

        if (metadata?.messageType === 'run-graph' && codeIdRef.current) {
          const provider = selectedLlmProviderRef.current
          const enabledMcpServers = enabledMcpServersRef.current
          const customMcpServers = enabledCustomServersRef.current

          return {
            api: `${agentUrlRef.current}/graph/${codeIdRef.current}/run`,
            body: {
              provider: provider?.type,
              providerType: provider?.type,
              providerName: provider?.name,
              model: provider?.modelId ?? 'browseros',
              contextWindowSize: provider?.contextWindow,
              temperature: provider?.temperature,
              resourceName: provider?.resourceName,
              // Bedrock-specific
              accessKeyId: provider?.accessKeyId,
              secretAccessKey: provider?.secretAccessKey,
              region: provider?.region,
              sessionToken: provider?.sessionToken,
              apiKey: provider?.apiKey,
              baseUrl: provider?.baseUrl,
              browserContext: {
                windowId: metadata?.window?.id,
                activeTab: metadata?.window?.tabs?.[0],
                enabledMcpServers: compact(enabledMcpServers),
                customMcpServers,
              },
              userSystemPrompt: personalizationRef.current,
            },
          }
        }

        return {
          api: `${agentUrlRef.current}/graph`,
          body: {
            query: lastMessageText,
          },
        }
      },
    }),
  })

  const lastAssistantMessageWithGraph = messages.findLast((m) => {
    if (m.role !== 'assistant') return false
    const metadata = m.metadata as GraphMessageMetadata | undefined
    return metadata?.graph !== undefined
  })

  const onClickTest = async () => {
    let backgroundWindow: chrome.windows.Window | undefined
    try {
      backgroundWindow = await chrome.windows.create({
        url: 'chrome://newtab',
        focused: true,
        type: 'normal',
      })
    } catch {
      // Fallback when no window context is available (e.g. all windows closed)
      const tab = await chrome.tabs.create({
        url: 'chrome://newtab',
        active: true,
      })
      if (tab.windowId) {
        backgroundWindow = await chrome.windows.get(tab.windowId)
      }
    }

    sendMessage({
      text: 'Run a test of the graph you just created.',
      metadata: {
        messageType: 'run-graph' as MessageType,
        codeId,
        window: backgroundWindow,
      },
    })
  }

  const hasUnsavedChanges = savedWorkflowId ? codeId !== savedCodeId : true
  const shouldBlockNavigation = !!codeId && hasUnsavedChanges

  // Handle browser refresh/close
  useEffect(() => {
    const handleBeforeUnload = (e: BeforeUnloadEvent) => {
      if (shouldBlockNavigation) {
        e.preventDefault()
      }
    }

    window.addEventListener('beforeunload', handleBeforeUnload)
    return () => window.removeEventListener('beforeunload', handleBeforeUnload)
  }, [shouldBlockNavigation])

  const onClickSave = async () => {
    if (!graphName || !codeId) return

    if (savedWorkflowId) {
      await editWorkflow(savedWorkflowId, {
        workflowName: graphName,
        codeId,
      })
      setSavedCodeId(codeId)
    } else {
      const newWorkflow = await addWorkflow({
        workflowName: graphName,
        codeId,
      })
      setSavedWorkflowId(newWorkflow.id)
      setSavedCodeId(codeId)
    }
    track(GRAPH_SAVED_EVENT)
  }

  // Provider data for header
  const providers: Provider[] = llmProviders.map((p) => ({
    id: p.id,
    name: p.name,
    type: p.type,
  }))

  const selectedProviderForHeader: Provider | undefined = selectedLlmProvider
    ? {
        id: selectedLlmProvider.id,
        name: selectedLlmProvider.name,
        type: selectedLlmProvider.type,
      }
    : providers[0]

  // Has generated code but can't auto-save (no name)
  const hasUnsavedWork = codeId && !graphName

  const resetToNewWorkflow = () => {
    setCodeId(undefined)
    setGraphData(undefined)
    setGraphName('')
    setSavedWorkflowId(undefined)
    setSavedCodeId(undefined)
    setMessages([])
  }

  const handleSelectProvider = (provider: Provider) => {
    setDefaultProvider(provider.id)
  }

  const handleNewWorkflow = async () => {
    // Can auto-save: has name AND code
    if (graphName && codeId) {
      await onClickSave()
      resetToNewWorkflow()
      return
    }

    // Has unsaved work that can't be auto-saved: show confirmation
    if (hasUnsavedWork) {
      setShowDiscardDialog(true)
      return
    }

    // Nothing to save, just reset
    resetToNewWorkflow()
  }

  const handleConfirmDiscard = () => {
    setShowDiscardDialog(false)
    resetToNewWorkflow()
  }

  const handleSuggestionClick = (prompt: string) => {
    sendMessage({
      text: prompt,
      metadata: {
        messageType: 'create-graph' as MessageType,
      },
    })
  }

  useDeepCompareEffect(() => {
    if (status === 'ready' && lastAssistantMessageWithGraph) {
      const metadata = lastAssistantMessageWithGraph.metadata as
        | GraphMessageMetadata
        | undefined
      setCodeId(metadata?.codeId)
      setGraphData(metadata?.graph)
    }
  }, [status, lastAssistantMessageWithGraph ?? {}])

  if (!isInitialized || isLoadingProviders || !selectedProviderForHeader) {
    return (
      <div className="flex h-screen w-screen items-center justify-center bg-background text-foreground">
        <div className="fade-in animate-in text-muted-foreground duration-200 [animation-delay:300ms] [animation-fill-mode:backwards]">
          Loading...
        </div>
      </div>
    )
  }

  return (
    <div className="h-screen w-screen bg-background text-foreground">
      <ResizablePanelGroup orientation="horizontal">
        <ResizablePanel
          id="graph-canvas"
          defaultSize={'70%'}
          minSize={'30%'}
          maxSize={'70%'}
          onResize={(size) => setCanvasPanelSize(size)}
        >
          <GraphCanvas
            graphName={graphName}
            onGraphNameChange={(val) => setGraphName(val)}
            graphData={graphData}
            codeId={codeId}
            onClickTest={onClickTest}
            onClickSave={onClickSave}
            isSaved={!!savedWorkflowId}
            hasUnsavedChanges={hasUnsavedChanges}
            shouldBlockNavigation={shouldBlockNavigation}
            panelSize={canvasPanelSize}
          />
        </ResizablePanel>

        <ResizableHandle withHandle />

        <ResizablePanel
          id="graph-chat"
          defaultSize={'30%'}
          maxSize={'70%'}
          minSize={'30%'}
        >
          <div className="flex h-full flex-col">
            <WorkflowsChatHeader
              selectedProvider={selectedProviderForHeader}
              providers={providers}
              onSelectProvider={handleSelectProvider}
              onNewWorkflow={handleNewWorkflow}
              hasMessages={messages.length > 0}
            />
            <div className="min-h-0 flex-1">
              <GraphChat
                messages={messages}
                onSubmit={onSubmit}
                onInputChange={updateQuery}
                onStop={stop}
                input={query}
                status={status}
                agentUrlError={agentUrlError}
                chatError={error}
                onSuggestionClick={handleSuggestionClick}
              />
            </div>
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>

      <AlertDialog open={showDiscardDialog} onOpenChange={setShowDiscardDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard unsaved workflow?</AlertDialogTitle>
            <AlertDialogDescription>
              You have an unsaved workflow. Creating a new one will discard your
              current changes.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={handleConfirmDiscard}>
              Discard
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
