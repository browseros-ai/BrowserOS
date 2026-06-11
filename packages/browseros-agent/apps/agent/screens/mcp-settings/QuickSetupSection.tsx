import { Check, Copy, Terminal } from 'lucide-react'
import { type FC, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'

export interface QuickSetupSectionProps {
  serverUrl: string | null
}

interface ClientConfig {
  id: string
  name: string
  type: 'command' | 'json' | 'generic'
  getSnippet: (url: string) => string
  fileName?: string
  /** Short note rendered under the snippet to set expectations. */
  notes?: string
}

const clients: ClientConfig[] = [
  {
    id: 'generic',
    name: 'Other agents',
    type: 'generic',
    fileName: undefined,
    getSnippet: (url) =>
      JSON.stringify(
        {
          mcpServers: {
            browseros: {
              type: 'http',
              url,
            },
          },
        },
        null,
        2,
      ),
    notes:
      "Most MCP-capable agents accept this config block. Drop it into the agent's MCP settings file (often `mcp.json`, `settings.json`, or similar) and restart the agent. If your agent uses a different config schema, the load-bearing values are the URL above and the `http` transport.",
  },
  {
    id: 'claude-code',
    name: 'Claude Code',
    type: 'command',
    getSnippet: (url) =>
      `claude mcp add --transport http browseros ${url} --scope user`,
  },
  {
    id: 'codex',
    name: 'Codex',
    type: 'command',
    getSnippet: (url) => `codex mcp add browseros ${url}`,
  },
  {
    id: 'claude-desktop',
    name: 'Claude Desktop',
    type: 'json',
    fileName: 'claude_desktop_config.json',
    getSnippet: (url) =>
      JSON.stringify(
        {
          mcpServers: {
            browserOS: {
              command: 'npx',
              args: ['mcp-remote', url],
            },
          },
        },
        null,
        2,
      ),
  },
]

const CopyButton: FC<{ text: string }> = ({ text }) => {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // Clipboard API failed
    }
  }

  return (
    <Button
      variant="ghost"
      size="icon-sm"
      onClick={handleCopy}
      className="shrink-0 text-muted-foreground hover:text-foreground"
    >
      {copied ? (
        <Check className="h-3.5 w-3.5 text-green-600" />
      ) : (
        <Copy className="h-3.5 w-3.5" />
      )}
    </Button>
  )
}

export const QuickSetupSection: FC<QuickSetupSectionProps> = ({
  serverUrl,
}) => {
  if (!serverUrl) return null

  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <Terminal className="h-6 w-6 text-[var(--accent-orange)]" />
        </div>
        <div className="flex-1">
          <h2 className="mb-1 font-semibold text-xl">Manual setup</h2>
          <p className="mb-3 text-muted-foreground text-sm">
            Pick your agent below for a copy-paste command. For agents BrowserOS
            does not list above, use the <strong>Other agents</strong> tab — the
            URL + HTTP transport is what every MCP-capable client needs, just
            dropped into whatever config shape that client uses.
          </p>

          <div className="mb-3 flex items-start gap-2 rounded-lg border border-border bg-background px-3 py-2.5">
            <span className="font-mono text-muted-foreground text-xs">
              MCP URL
            </span>
            <pre className="flex-1 overflow-x-auto whitespace-pre-wrap break-all font-mono text-xs">
              {serverUrl}
            </pre>
            <CopyButton text={serverUrl} />
          </div>

          <Tabs defaultValue="generic">
            <TabsList className="mb-3 flex-wrap">
              {clients.map((client) => (
                <TabsTrigger key={client.id} value={client.id}>
                  {client.name}
                </TabsTrigger>
              ))}
            </TabsList>

            {clients.map((client) => {
              const snippet = client.getSnippet(serverUrl)
              return (
                <TabsContent key={client.id} value={client.id}>
                  <div className="space-y-3">
                    {client.fileName && (
                      <p className="text-muted-foreground text-xs">
                        Add to{' '}
                        <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs">
                          {client.fileName}
                        </code>
                      </p>
                    )}
                    <div className="flex items-start gap-2 rounded-lg border border-border bg-background px-3 py-2.5">
                      <pre className="flex-1 overflow-x-auto whitespace-pre-wrap break-all font-mono text-xs">
                        {client.type === 'command' && (
                          <span className="mr-1 text-muted-foreground">$</span>
                        )}
                        {snippet}
                      </pre>
                      <CopyButton text={snippet} />
                    </div>
                    {client.notes && (
                      <p className="text-muted-foreground text-xs leading-relaxed">
                        {client.notes}
                      </p>
                    )}
                  </div>
                </TabsContent>
              )
            })}
          </Tabs>
        </div>
      </div>
    </div>
  )
}
