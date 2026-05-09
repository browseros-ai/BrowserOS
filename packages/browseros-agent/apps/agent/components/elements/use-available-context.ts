import { useEffect, useMemo, useState } from 'react'
import { getAgentServerUrl } from '@/lib/browseros/helpers'
import type { ContextAttachment } from '@/lib/context-attachments'
import { useWorkspace } from '@/lib/workspace/use-workspace'
import { useAvailableTabs } from './use-available-tabs'

interface UseAvailableContextOptions {
  enabled: boolean
  filterText?: string
  includeAttachments?: boolean
}

export type ContextPickerItem =
  | { type: 'tab'; tab: chrome.tabs.Tab }
  | { type: 'file'; attachment: ContextAttachment }
  | { type: 'memory'; attachment: ContextAttachment }

interface ContextSearchResponse {
  files?: ContextAttachment[]
  memories?: ContextAttachment[]
}

export function useAvailableContext({
  enabled,
  filterText = '',
  includeAttachments = true,
}: UseAvailableContextOptions): {
  tabs: chrome.tabs.Tab[]
  files: ContextAttachment[]
  memories: ContextAttachment[]
  allTabs: chrome.tabs.Tab[]
  isLoading: boolean
  hasWorkspace: boolean
  items: ContextPickerItem[]
} {
  const { selectedFolder } = useWorkspace()
  const {
    tabs,
    allTabs,
    isLoading: isLoadingTabs,
  } = useAvailableTabs({ enabled, filterText })
  const [files, setFiles] = useState<ContextAttachment[]>([])
  const [memories, setMemories] = useState<ContextAttachment[]>([])
  const [isLoadingContext, setIsLoadingContext] = useState(false)

  useEffect(() => {
    if (!enabled || !includeAttachments) {
      setFiles([])
      setMemories([])
      setIsLoadingContext(false)
      return
    }

    const controller = new AbortController()
    const timeout = window.setTimeout(() => {
      setIsLoadingContext(true)
      void loadContextResults({
        query: filterText,
        cwd: selectedFolder?.path,
        signal: controller.signal,
      })
        .then((result) => {
          if (controller.signal.aborted) return
          setFiles(result.files ?? [])
          setMemories(result.memories ?? [])
        })
        .catch(() => {
          if (controller.signal.aborted) return
          setFiles([])
          setMemories([])
        })
        .finally(() => {
          if (!controller.signal.aborted) setIsLoadingContext(false)
        })
    }, 150)

    return () => {
      window.clearTimeout(timeout)
      controller.abort()
    }
  }, [enabled, filterText, includeAttachments, selectedFolder?.path])

  const items = useMemo<ContextPickerItem[]>(
    () => [
      ...tabs.map((tab) => ({ type: 'tab' as const, tab })),
      ...files.map((attachment) => ({
        type: 'file' as const,
        attachment,
      })),
      ...memories.map((attachment) => ({
        type: 'memory' as const,
        attachment,
      })),
    ],
    [files, memories, tabs],
  )

  return {
    tabs,
    files,
    memories,
    allTabs,
    isLoading: isLoadingTabs || isLoadingContext,
    hasWorkspace: Boolean(selectedFolder?.path),
    items,
  }
}

async function loadContextResults({
  query,
  cwd,
  signal,
}: {
  query: string
  cwd?: string
  signal: AbortSignal
}): Promise<ContextSearchResponse> {
  const baseUrl = await getAgentServerUrl()
  const params = new URLSearchParams()
  if (query) params.set('q', query)

  const memoryUrl = `${baseUrl}/context/memories?${params.toString()}`
  const memoryPromise = fetch(memoryUrl, { signal }).then((response) =>
    response.ok ? response.json() : { memories: [] },
  )

  const filePromise = cwd
    ? (() => {
        const fileParams = new URLSearchParams(params)
        fileParams.set('cwd', cwd)
        return fetch(`${baseUrl}/context/files?${fileParams.toString()}`, {
          signal,
        }).then((response) => (response.ok ? response.json() : { files: [] }))
      })()
    : Promise.resolve({ files: [] })

  const [fileResult, memoryResult] = (await Promise.all([
    filePromise,
    memoryPromise,
  ])) as ContextSearchResponse[]

  return {
    files: fileResult.files ?? [],
    memories: memoryResult.memories ?? [],
  }
}
