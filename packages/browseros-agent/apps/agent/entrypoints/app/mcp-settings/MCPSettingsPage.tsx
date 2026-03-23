import { type FC, useCallback, useEffect, useState } from 'react'
import { getMcpServerUrl } from '@/lib/browseros/helpers'
import type { McpTool } from '@/lib/mcp/client'
import { sendServerMessage } from '@/lib/messaging/server/serverMessages'
import { MCPServerHeader } from './MCPServerHeader'
import { MCPToolsSection } from './MCPToolsSection'
import { QuickSetupSection } from './QuickSetupSection'

/** @public */
export const MCPSettingsPage: FC = () => {
  const [serverUrl, setServerUrl] = useState<string | null>(null)
  const [urlLoading, setUrlLoading] = useState(true)
  const [urlError, setUrlError] = useState<string | null>(null)

  const [tools, setTools] = useState<McpTool[]>([])
  const [toolsLoading, setToolsLoading] = useState(false)
  const [toolsError, setToolsError] = useState<string | null>(null)

  const loadServerUrlAndTools = useCallback(async () => {
    let url: string | null = null
    setUrlLoading(true)
    setUrlError(null)
    setToolsError(null)

    try {
      url = await getMcpServerUrl()
      setServerUrl(url)
      setUrlLoading(false)

      setToolsLoading(true)
      const result = await sendServerMessage('fetchMcpTools', undefined)
      if (result.error) {
        setToolsError(result.error)
      } else {
        setTools(result.tools)
      }
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : 'Failed to load'
      if (!url) {
        setUrlError(errorMsg)
      } else {
        setToolsError(errorMsg)
      }
    } finally {
      setUrlLoading(false)
      setToolsLoading(false)
    }
  }, [])

  useEffect(() => {
    loadServerUrlAndTools()
  }, [loadServerUrlAndTools])

  const handleRefreshTools = async () => {
    if (!serverUrl) return

    setToolsLoading(true)
    setToolsError(null)
    setTools([])

    try {
      const result = await sendServerMessage('fetchMcpTools', undefined)
      if (result.error) {
        setToolsError(result.error)
      } else {
        setTools(result.tools)
      }
    } catch (err) {
      setToolsError(
        err instanceof Error ? err.message : 'Failed to fetch tools',
      )
    } finally {
      setToolsLoading(false)
    }
  }

  return (
    <div className="fade-in slide-in-from-bottom-5 animate-in space-y-6 duration-500">
      <MCPServerHeader
        serverUrl={serverUrl}
        isLoading={urlLoading}
        error={urlError}
        onServerRestart={loadServerUrlAndTools}
      />

      <QuickSetupSection serverUrl={serverUrl} />

      <MCPToolsSection
        tools={tools}
        isLoading={toolsLoading}
        error={toolsError}
        onRefresh={handleRefreshTools}
      />
    </div>
  )
}
