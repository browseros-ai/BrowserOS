import { useChat } from '@ai-sdk/react'
import { DefaultChatTransport } from 'ai'
import { compact } from 'es-toolkit/array'
import { useEffect, useRef, useState } from 'react'
import { useChatRefs } from '@/entrypoints/sidepanel/index/useChatRefs'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import {
  WORKFLOW_RUN_COMPLETED_EVENT,
  WORKFLOW_RUN_RETRIED_EVENT,
  WORKFLOW_RUN_STOPPED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'

type WorkflowMessageMetadata = {
  window?: chrome.windows.Window
}

export const useRunWorkflow = () => {
  const [isRunning, setIsRunning] = useState(false)
  const [runningWorkflowName, setRunningWorkflowName] = useState<string>('')
  const [wasCancelled, setWasCancelled] = useState(false)
  const codeIdRef = useRef<string | undefined>(undefined)

  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  const {
    selectedLlmProviderRef,
    enabledMcpServersRef,
    enabledCustomServersRef,
    personalizationRef,
  } = useChatRefs()

  const agentUrlRef = useRef(agentServerUrl)

  useEffect(() => {
    agentUrlRef.current = agentServerUrl
  }, [agentServerUrl])

  const { sendMessage, stop, status, messages, setMessages, error } = useChat({
    transport: new DefaultChatTransport({
      prepareSendMessagesRequest: async ({ messages }) => {
        const lastMessage = messages[messages.length - 1]
        const metadata = lastMessage.metadata as
          | WorkflowMessageMetadata
          | undefined
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
            supportsImages: provider?.supportsImages,
          },
        }
      },
    }),
  })

  const previousStatus = useRef(status)
  useEffect(() => {
    const wasProcessing =
      previousStatus.current === 'streaming' ||
      previousStatus.current === 'submitted'
    const justFinished =
      wasProcessing && (status === 'ready' || status === 'error')

    if (justFinished && isRunning) {
      track(WORKFLOW_RUN_COMPLETED_EVENT, {
        status: wasCancelled
          ? 'cancelled'
          : status === 'error'
            ? 'failed'
            : 'completed',
      })
    }
    previousStatus.current = status
  }, [status, isRunning, wasCancelled])

  const startWorkflowRun = async () => {
    setMessages([])
    setWasCancelled(false)

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
      text: 'Run the workflow.',
      metadata: {
        window: backgroundWindow,
      },
    })
  }

  const runWorkflow = async (codeId: string, workflowName: string) => {
    codeIdRef.current = codeId
    setRunningWorkflowName(workflowName)
    setIsRunning(true)
    await startWorkflowRun()
  }

  const stopRun = () => {
    track(WORKFLOW_RUN_STOPPED_EVENT)
    setWasCancelled(true)
    stop()
  }

  const retry = async () => {
    track(WORKFLOW_RUN_RETRIED_EVENT)
    await startWorkflowRun()
  }

  const closeDialog = () => {
    setIsRunning(false)
    setRunningWorkflowName('')
    setWasCancelled(false)
    setMessages([])
  }

  return {
    isRunning,
    runningWorkflowName,
    messages,
    status,
    wasCancelled,
    error,
    runWorkflow,
    stopRun,
    retry,
    closeDialog,
  }
}
