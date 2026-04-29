import { Bot, Cpu, Loader2, MessageSquare, Plus, Trash2 } from 'lucide-react'
import type { FC } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import type { AgentListItem } from './agents-page-types'

interface AgentListProps {
  agents: AgentListItem[]
  loading: boolean
  deletingAgentKey: string | null
  onChatAgent: (agent: AgentListItem) => void
  onCreateAgent: () => void
  onDeleteAgent: (agent: AgentListItem) => void
}

export const AgentList: FC<AgentListProps> = ({
  agents,
  loading,
  deletingAgentKey,
  onChatAgent,
  onCreateAgent,
  onDeleteAgent,
}) => {
  if (loading && agents.length === 0) {
    return (
      <div className="flex h-36 items-center justify-center rounded-lg border border-border/70">
        <Loader2 className="size-5 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (agents.length === 0) {
    return (
      <Card>
        <CardContent className="flex h-48 flex-col items-center justify-center gap-4 text-center">
          <div className="flex size-10 items-center justify-center rounded-lg bg-muted text-muted-foreground">
            <Bot className="size-5" />
          </div>
          <div className="space-y-1">
            <h2 className="font-medium text-base">No agents</h2>
            <p className="text-muted-foreground text-sm">
              Create an OpenClaw, Claude Code, or Codex agent.
            </p>
          </div>
          <Button variant="outline" onClick={onCreateAgent}>
            <Plus className="mr-2 size-4" />
            New Agent
          </Button>
        </CardContent>
      </Card>
    )
  }

  return (
    <div className="grid gap-3">
      {agents.map((agent) => (
        <Card key={agent.key} className="rounded-lg border-border/70">
          <CardHeader className="flex flex-row items-center justify-between gap-4 py-3">
            <div className="flex min-w-0 items-center gap-3">
              <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                {agent.source === 'openclaw' ? (
                  <Cpu className="size-5" />
                ) : (
                  <Bot className="size-5" />
                )}
              </div>
              <div className="min-w-0">
                <CardTitle className="truncate text-base">
                  {agent.name}
                </CardTitle>
                <div className="mt-1 flex flex-wrap items-center gap-2 text-muted-foreground text-xs">
                  <Badge variant="outline" className="rounded-md">
                    {agent.runtimeLabel}
                  </Badge>
                  <span>{agent.modelLabel}</span>
                  <Badge variant="outline" className="rounded-md">
                    main
                  </Badge>
                </div>
                <p className="mt-1 truncate font-mono text-muted-foreground text-xs">
                  {agent.detail}
                </p>
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-1">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => onChatAgent(agent)}
                disabled={!agent.canChat}
              >
                <MessageSquare className="mr-1 size-4" />
                Chat
              </Button>
              {agent.canDelete ? (
                <Button
                  variant="ghost"
                  size="icon"
                  title="Delete agent"
                  onClick={() => onDeleteAgent(agent)}
                  disabled={deletingAgentKey === agent.key}
                >
                  {deletingAgentKey === agent.key ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <Trash2 className="size-4 text-destructive" />
                  )}
                </Button>
              ) : null}
            </div>
          </CardHeader>
        </Card>
      ))}
    </div>
  )
}
