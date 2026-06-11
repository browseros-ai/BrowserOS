import {
  AlertCircle,
  ChevronDown,
  Download,
  ExternalLink,
  Loader2,
  Plug,
} from 'lucide-react'
import { type FC, useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { presentationFor } from './integrations-section.helpers'
import {
  type McpAgentRow,
  useInstallAgent,
  useMcpAgents,
  useUninstallAgent,
} from './integrations-section.hooks'
import { QuickSetupSection } from './QuickSetupSection'

interface IntegrationsSectionProps {
  serverUrl: string | null
}

export const IntegrationsSection: FC<IntegrationsSectionProps> = ({
  serverUrl,
}) => {
  const agentsQuery = useMcpAgents()
  const install = useInstallAgent()
  const uninstall = useUninstallAgent()
  const [errors, setErrors] = useState<Record<string, string | null>>({})

  const handleInstall = async (agentId: string) => {
    setErrors((prev) => ({ ...prev, [agentId]: null }))
    try {
      const result = await install.mutateAsync(agentId)
      if (!result.success) {
        setErrors((prev) => ({
          ...prev,
          [agentId]: result.message ?? 'Install failed.',
        }))
      }
    } catch (err) {
      setErrors((prev) => ({
        ...prev,
        [agentId]: err instanceof Error ? err.message : String(err),
      }))
    }
  }

  const handleUninstall = async (agentId: string) => {
    setErrors((prev) => ({ ...prev, [agentId]: null }))
    try {
      const result = await uninstall.mutateAsync(agentId)
      if (!result.success) {
        setErrors((prev) => ({
          ...prev,
          [agentId]: result.message ?? 'Uninstall failed.',
        }))
      }
    } catch (err) {
      setErrors((prev) => ({
        ...prev,
        [agentId]: err instanceof Error ? err.message : String(err),
      }))
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <Plug className="h-6 w-6 text-[var(--accent-orange)]" />
        </div>
        <div className="flex-1">
          <h2 className="mb-1 font-semibold text-xl">Integrations</h2>
          <p className="text-muted-foreground text-sm">
            One-click install of BrowserOS as MCP into your installed AI agents.
            Click Install to add BrowserOS to the agent's config; click
            Uninstall to remove it.
          </p>
        </div>
      </div>

      {agentsQuery.isLoading && (
        <Card className="flex items-center gap-3 p-4">
          <Loader2 className="h-4 w-4 animate-spin" />
          <span className="text-muted-foreground text-sm">
            Detecting installed agents...
          </span>
        </Card>
      )}

      {agentsQuery.isError && (
        <Card className="flex items-start gap-3 border-destructive/40 bg-destructive/5 p-4">
          <AlertCircle className="mt-0.5 h-4 w-4 text-destructive" />
          <div className="space-y-1 text-sm">
            <div className="font-medium text-destructive">
              Could not load agents
            </div>
            <div className="text-muted-foreground">
              {agentsQuery.error instanceof Error
                ? agentsQuery.error.message
                : String(agentsQuery.error)}
            </div>
            <Button
              size="sm"
              variant="outline"
              className="mt-2"
              onClick={() => agentsQuery.refetch()}
            >
              Retry
            </Button>
          </div>
        </Card>
      )}

      {agentsQuery.data && agentsQuery.data.length === 0 && (
        <Card className="p-4 text-muted-foreground text-sm">
          No supported agents found on this system.
        </Card>
      )}

      {agentsQuery.data && agentsQuery.data.length > 0 && (
        <div className="space-y-2">
          {agentsQuery.data.map((agent) => (
            <AgentRow
              key={agent.id}
              agent={agent}
              busy={
                (install.isPending && install.variables === agent.id) ||
                (uninstall.isPending && uninstall.variables === agent.id)
              }
              error={errors[agent.id] ?? null}
              onInstall={handleInstall}
              onUninstall={handleUninstall}
            />
          ))}
        </div>
      )}

      <Collapsible>
        <CollapsibleTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            className="mt-2 text-muted-foreground hover:text-foreground"
          >
            Show manual setup commands
            <ChevronDown className="ml-1 h-3 w-3" />
          </Button>
        </CollapsibleTrigger>
        <CollapsibleContent className="pt-3">
          <QuickSetupSection serverUrl={serverUrl} />
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}

interface AgentRowProps {
  agent: McpAgentRow
  busy: boolean
  error: string | null
  onInstall: (id: string) => void
  onUninstall: (id: string) => void
}

const AgentRow: FC<AgentRowProps> = ({
  agent,
  busy,
  error,
  onInstall,
  onUninstall,
}) => {
  const presentation = presentationFor(agent.id)

  return (
    <Card className="p-4">
      <div className="flex items-center gap-4">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-muted">
          <span className="font-semibold text-sm">
            {presentation.label.charAt(0)}
          </span>
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="font-semibold">{presentation.label}</span>
            {agent.linked && (
              <Badge
                variant="secondary"
                className="rounded bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]"
              >
                INSTALLED
              </Badge>
            )}
            {!agent.installed && (
              <Badge variant="outline" className="rounded text-xs">
                NOT DETECTED
              </Badge>
            )}
          </div>
          <p className="truncate text-muted-foreground text-sm">
            {agent.installed
              ? `Connect ${presentation.label} to BrowserOS so it can use your tools.`
              : `Install ${presentation.label} to connect it to BrowserOS.`}
          </p>
          {error && <p className="mt-1 text-destructive text-xs">{error}</p>}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {!agent.installed && (
            <Button asChild variant="outline" size="sm">
              <a
                href={presentation.installUrl}
                target="_blank"
                rel="noopener noreferrer"
              >
                Get {presentation.label}
                <ExternalLink className="ml-1 h-3 w-3" />
              </a>
            </Button>
          )}
          {agent.installed && !agent.linked && (
            <Button
              size="sm"
              disabled={busy}
              onClick={() => onInstall(agent.id)}
            >
              {busy && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
              {!busy && <Download className="mr-1.5 h-4 w-4" />}
              Install
            </Button>
          )}
          {agent.installed && agent.linked && (
            <Button
              size="sm"
              variant="destructive"
              disabled={busy}
              onClick={() => onUninstall(agent.id)}
            >
              {busy && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
              Uninstall
            </Button>
          )}
        </div>
      </div>
    </Card>
  )
}
