import { Check, Copy, ExternalLink, Globe, Server } from 'lucide-react'
import { type FC, useState } from 'react'
import { Button } from '@/components/ui/button'
import { mcpClientsHelpUrl } from '@/lib/constants/productUrls'

interface MCPServerHeaderProps {
  serverUrl: string | null
  isLoading: boolean
  error: string | null
  title?: string
  description?: string
  remoteAccessEnabled?: boolean
}

export const MCPServerHeader: FC<MCPServerHeaderProps> = ({
  serverUrl,
  isLoading,
  error,
  title = 'BrowserOS MCP Server',
  description = 'Connect BrowserOS to MCP clients like claude code, gemini and others.',
  remoteAccessEnabled = false,
}) => {
  const [isCopied, setIsCopied] = useState(false)

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

  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <Server className="h-6 w-6 text-[var(--accent-orange)]" />
        </div>
        <div className="flex-1">
          <div className="mb-1 flex items-center justify-between">
            <h2 className="font-semibold text-xl">{title}</h2>
            <a
              href={mcpClientsHelpUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-1 text-muted-foreground text-sm transition-colors hover:text-[var(--accent-orange)]"
            >
              Setup a client
              <ExternalLink className="h-3.5 w-3.5" />
            </a>
          </div>
          <p className="mb-6 text-muted-foreground text-sm">{description}</p>

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
              >
                {isCopied ? (
                  <Check className="h-4 w-4 text-green-600" />
                ) : (
                  <Copy className="h-4 w-4" />
                )}
              </Button>
            </div>
          </div>

          {remoteAccessEnabled && serverUrl && !isLoading && (
            <div className="mt-3 flex items-start gap-2 rounded-lg bg-muted/50 px-3 py-2">
              <Globe className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <p className="text-muted-foreground text-xs">
                External access is enabled. To connect from another device,
                replace <span className="font-mono">127.0.0.1</span> with this
                machine's IP address.
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
