import { ArrowRight, Bot, Plus, Settings2 } from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
import { useNavigate } from 'react-router'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'
import type { AgentEntry } from '@/entrypoints/app/agents/useOpenClaw'
import { ImportDataHint } from '@/entrypoints/newtab/index/ImportDataHint'
import { SignInHint } from '@/entrypoints/newtab/index/SignInHint'
import { useActiveHint } from '@/entrypoints/newtab/index/useActiveHint'
import type { AgentCardData } from '@/lib/agent-conversations/types'
import { AgentCardDock } from './AgentCardDock'
import { useAgentCommandData } from './agent-command-layout'
import { ConversationInput } from './ConversationInput'
import { buildAgentCardData } from './useAgentCardData'
import { useAgentDashboard } from './useAgentDashboard'

function AgentCommandSetupState({
  onOpenAgents,
}: {
  onOpenAgents: () => void
}) {
  return (
    <Card className="border-border/60 bg-card/90 shadow-sm">
      <CardContent className="flex flex-col items-center gap-4 p-8 text-center">
        <div className="flex size-12 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
          <Bot className="size-5" />
        </div>
        <div className="space-y-2">
          <h2 className="font-semibold text-lg">Set up your first agent</h2>
          <p className="max-w-md text-muted-foreground text-sm leading-6">
            Connect OpenClaw and create an agent before using the new tab as
            your workspace.
          </p>
        </div>
        <Button onClick={onOpenAgents} className="gap-2 rounded-xl">
          Open Agent Setup
          <ArrowRight className="size-4" />
        </Button>
      </CardContent>
    </Card>
  )
}

function EmptyAgentsState({ onOpenAgents }: { onOpenAgents: () => void }) {
  return (
    <Card className="border-border/60 bg-card/90 shadow-sm">
      <CardContent className="flex flex-col items-center gap-4 p-8 text-center">
        <div className="flex size-12 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
          <Plus className="size-5" />
        </div>
        <div className="space-y-2">
          <h2 className="font-semibold text-lg">No agents yet</h2>
          <p className="max-w-md text-muted-foreground text-sm leading-6">
            Create an agent to start using BrowserOS as an agent-first new tab.
          </p>
        </div>
        <Button variant="outline" onClick={onOpenAgents} className="rounded-xl">
          Create agent
        </Button>
      </CardContent>
    </Card>
  )
}

function OpenClawUnavailableState({
  onOpenAgents,
}: {
  onOpenAgents: () => void
}) {
  return (
    <Card className="border-border/60 bg-card/90 shadow-sm">
      <CardContent className="flex flex-col items-center gap-4 p-8 text-center">
        <div className="flex size-12 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
          <Settings2 className="size-5" />
        </div>
        <div className="space-y-2">
          <h2 className="font-semibold text-lg">OpenClaw is unavailable</h2>
          <p className="max-w-md text-muted-foreground text-sm leading-6">
            Review your agent setup to restart the gateway or reconnect the
            local service.
          </p>
        </div>
        <Button onClick={onOpenAgents} className="gap-2 rounded-xl">
          Open Agent Setup
          <ArrowRight className="size-4" />
        </Button>
      </CardContent>
    </Card>
  )
}

function RecentThreads({
  activeAgentId,
  agents,
  onOpenAgents,
  onSelectAgent,
}: {
  activeAgentId?: string | null
  agents: AgentCardData[]
  onOpenAgents: () => void
  onSelectAgent: (agentId: string) => void
}) {
  if (agents.length === 0) return null

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h2 className="font-semibold text-base">Recent agents</h2>
          <p className="text-muted-foreground text-sm">
            Continue from where you left off.
          </p>
        </div>
        <Button
          variant="outline"
          onClick={onOpenAgents}
          className="rounded-xl"
          size="sm"
        >
          Manage agents
        </Button>
      </div>
      <AgentCardDock
        agents={agents}
        activeAgentId={activeAgentId ?? undefined}
        onSelectAgent={onSelectAgent}
        onCreateAgent={onOpenAgents}
      />
    </section>
  )
}

export const AgentCommandHome: FC = () => {
  const navigate = useNavigate()
  const activeHint = useActiveHint()
  const { status, agents } = useAgentCommandData()
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null)
  const { data: dashboard } = useAgentDashboard(status?.status === 'running')
  const cardData = buildAgentCardData(agents, status?.status, dashboard?.agents)

  useEffect(() => {
    if (agents.length === 0) {
      if (selectedAgentId) {
        setSelectedAgentId(null)
      }
      return
    }

    if (
      !selectedAgentId ||
      !agents.some((agent) => agent.agentId === selectedAgentId)
    ) {
      setSelectedAgentId(agents[0].agentId)
    }
  }, [agents, selectedAgentId])

  const handleSend = (input: { text: string }) => {
    if (!selectedAgentId) return
    // Home composer navigates to the conversation page with the prompt in
    // the query string. Attachments are dropped at this boundary in v1 —
    // the conversation page (where staging UX is most useful anyway) is
    // where users can attach. A future iteration can stash staged files
    // in chrome.storage.session and replay them on first mount there.
    navigate(
      `/home/agents/${selectedAgentId}?q=${encodeURIComponent(input.text)}`,
    )
  }

  const handleSelectAgent = (agent: AgentEntry) => {
    setSelectedAgentId(agent.agentId)
  }

  const openClawStatus = status?.status
  const isSetup = openClawStatus != null && openClawStatus !== 'uninitialized'
  const shouldShowUnavailableState =
    openClawStatus != null &&
    openClawStatus !== 'running' &&
    openClawStatus !== 'uninitialized' &&
    cardData.length === 0
  const selectedCard =
    cardData.find((agent) => agent.agentId === selectedAgentId) ?? cardData[0]

  return (
    <div className="min-h-full px-4 py-6">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-8">
        {isSetup ? (
          shouldShowUnavailableState ? (
            <OpenClawUnavailableState
              onOpenAgents={() => navigate('/agents')}
            />
          ) : cardData.length > 0 ? (
            <>
              <div className="flex flex-col items-center gap-5 pt-[max(10vh,24px)] text-center">
                <div className="space-y-3">
                  <h1 className="font-semibold text-[clamp(2rem,4vw,3.25rem)] leading-tight tracking-tight">
                    What should your agent work on next?
                  </h1>
                  <p className="mx-auto max-w-2xl text-muted-foreground text-sm leading-6">
                    Start with a task, continue a thread, or switch to another
                    agent without leaving the new tab.
                  </p>
                </div>

                <div className="w-full max-w-3xl">
                  <ConversationInput
                    variant="home"
                    agents={agents}
                    selectedAgentId={selectedAgentId}
                    onSelectAgent={handleSelectAgent}
                    onSend={handleSend}
                    onCreateAgent={() => navigate('/agents')}
                    streaming={false}
                    disabled={status?.status !== 'running'}
                    status={status?.status}
                    placeholder={
                      status?.status === 'running'
                        ? `Ask ${selectedCard?.name ?? 'your agent'} to handle a task...`
                        : 'OpenClaw is not running...'
                    }
                  />
                </div>
              </div>

              <Separator />

              <RecentThreads
                activeAgentId={selectedAgentId}
                agents={cardData}
                onOpenAgents={() => navigate('/agents')}
                onSelectAgent={(agentId) => navigate(`/home/agents/${agentId}`)}
              />
            </>
          ) : (
            <EmptyAgentsState onOpenAgents={() => navigate('/agents')} />
          )
        ) : (
          <AgentCommandSetupState onOpenAgents={() => navigate('/agents')} />
        )}
      </div>

      {activeHint === 'signin' ? <SignInHint /> : null}
      {activeHint === 'import' ? <ImportDataHint /> : null}
    </div>
  )
}
