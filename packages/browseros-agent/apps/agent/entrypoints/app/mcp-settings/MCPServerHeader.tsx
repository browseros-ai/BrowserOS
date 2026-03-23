import {
  Check,
  Copy,
  ExternalLink,
  Loader2,
  RefreshCw,
  Server,
} from 'lucide-react'
import { type FC, useCallback, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { MCP_SERVER_RESTARTED_EVENT } from '@/lib/constants/analyticsEvents'
import { sendServerMessage } from '@/lib/messaging/server/serverMessages'
import { track } from '@/lib/metrics/track'

interface MCPServerHeaderProps {
  serverUrl: string | null
  isLoading: boolean
  error: string | null
  onServerRestart?: () => void
}

const DOCS_URL = 'https://docs.browseros.com/features/use-with-claude-code'
const HEALTH_CHECK_TIMEOUT_MS = 60_000
const HEALTH_CHECK_INTERVAL_MS = 2_000

export const MCPServerHeader: FC<MCPServerHeaderProps> = ({
  serverUrl,
  isLoading,
  error,
  onServerRestart,
}) => {
  const [isCopied, setIsCopied] = useState(false)
  const [isRestarting, setIsRestarting] = useState(false)

  const handleCopy = async () => {
    if (!serverUrl) return
    try {
      await navigator.clipboard.writeText(serverUrl)
      setIsCopied(true)
      setTimeout(() => setIsCopied(false), 2000)
    } catch {
      // Clipboard API failed
    }
  }

  const checkServerHealth = useCallback(async (): Promise<boolean> => {
    try {
      const result = await sendServerMessage('checkHealth', undefined)
      return result.healthy
    } catch {
      return false
    }
  }, [])

  const handleRestart = async () => {
    setIsRestarting(true)
    try {
      const { getBrowserOSAdapter } = await import('@/lib/browseros/adapter')
      const { BROWSEROS_PREFS } = await import('@/lib/browseros/prefs')
      const adapter = getBrowserOSAdapter()
      await adapter.setPref(BROWSEROS_PREFS.RESTART_SERVER, true)

      const startTime = Date.now()
      const waitForHealth = (): Promise<boolean> =>
        new Promise((resolve) => {
          const check = async () => {
            if (Date.now() - startTime >= HEALTH_CHECK_TIMEOUT_MS) {
              resolve(false)
              return
            }
            if (await checkServerHealth()) {
              resolve(true)
              return
            }
            setTimeout(check, HEALTH_CHECK_INTERVAL_MS)
          }
          setTimeout(check, HEALTH_CHECK_INTERVAL_MS)
        })

      const healthy = await waitForHealth()
      if (healthy) {
        track(MCP_SERVER_RESTARTED_EVENT)
        toast.success('Server restarted successfully')
        onServerRestart?.()
      } else {
        toast.error('Server did not respond. Try restarting the browser.')
      }
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : 'Failed to restart server',
      )
    } finally {
      setIsRestarting(false)
    }
  }

  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <Server className="h-6 w-6 text-[var(--accent-orange)]" />
        </div>
        <div className="flex-1">
          <div className="mb-1 flex items-center justify-between">
            <h2 className="font-semibold text-xl">BrowserOS MCP Server</h2>
            <a
              href={DOCS_URL}
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-1 text-muted-foreground text-sm transition-colors hover:text-[var(--accent-orange)]"
            >
              Docs
              <ExternalLink className="h-3.5 w-3.5" />
            </a>
          </div>
          <p className="mb-6 text-muted-foreground text-sm">
            Connect BrowserOS to MCP clients like Claude Code, Gemini CLI and
            others.
          </p>

          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <span className="whitespace-nowrap font-medium text-sm">
              Server URL:
            </span>
            <div className="flex flex-1 items-center gap-2">
              <div className="flex-1 rounded-lg border border-border bg-background px-3 py-2 font-mono text-sm sm:max-w-md">
                {isLoading ? (
                  <span className="text-muted-foreground">Loading...</span>
                ) : error ? (
                  <span className="text-destructive">{error}</span>
                ) : (
                  serverUrl
                )}
              </div>
              <Button
                variant="outline"
                size="icon"
                onClick={handleCopy}
                disabled={!serverUrl || isLoading}
                className="shrink-0"
                title="Copy URL"
              >
                {isCopied ? (
                  <Check className="h-4 w-4 text-green-600" />
                ) : (
                  <Copy className="h-4 w-4" />
                )}
              </Button>
              <Button
                variant="outline"
                size="icon"
                onClick={handleRestart}
                disabled={isLoading || isRestarting}
                className="shrink-0"
                title="Restart server"
              >
                {isRestarting ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <RefreshCw className="h-4 w-4" />
                )}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
