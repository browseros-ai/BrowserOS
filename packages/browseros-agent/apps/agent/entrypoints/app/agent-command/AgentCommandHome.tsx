import { Plus } from 'lucide-react'
import { type FC, useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'
import type {
  HarnessAdapterDescriptor,
  HarnessAgent,
} from '@/entrypoints/app/agents/agent-harness-types'
import {
  useAgentAdapters,
  useHarnessAgents,
} from '@/entrypoints/app/agents/useAgents'
import type { AgentEntry } from '@/entrypoints/app/agents/useOpenClaw'
import { ImportDataHint } from '@/entrypoints/newtab/index/ImportDataHint'
import { SignInHint } from '@/entrypoints/newtab/index/SignInHint'
import { useActiveHint } from '@/entrypoints/newtab/index/useActiveHint'
import { AgentCardDock } from './AgentCardDock'
import { useAgentCommandData } from './agent-command-layout'
import {
  ConversationInput,
  type ConversationInputSendInput,
} from './ConversationInput'
import { orderHomeAgents } from './home-agent-card.helpers'
import { setPendingInitialMessage } from './pending-initial-message'

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

function RecentThreads({
  activeAgentId,
  agents,
  adapters,
  onOpenAgents,
  onSelectAgent,
}: {
  activeAgentId?: string | null
  agents: HarnessAgent[]
  adapters: HarnessAdapterDescriptor[]
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
        adapters={adapters}
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
  // The conversation input still consumes the merged AgentEntry list
  // from the layout context (handles legacy /claw/agents entries that
  // haven't yet been backfilled into the harness store). The Recent
  // Agents grid below reads the richer harness payload directly.
  const { agents: legacyAgents, status } = useAgentCommandData()
  const { harnessAgents } = useHarnessAgents()
  const { adapters } = useAgentAdapters()
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null)

  const orderedAgents = useMemo(
    () => orderHomeAgents(harnessAgents),
    [harnessAgents],
  )

  useEffect(() => {
    if (legacyAgents.length === 0) {
      if (selectedAgentId) setSelectedAgentId(null)
      return
    }
    if (
      !selectedAgentId ||
      !legacyAgents.some((agent) => agent.agentId === selectedAgentId)
    ) {
      setSelectedAgentId(legacyAgents[0].agentId)
    }
  }, [legacyAgents, selectedAgentId])

  const handleSend = (input: ConversationInputSendInput) => {
    if (!selectedAgentId) return
    // Stash text + attachments in the in-memory registry. Text also
    // travels in `?q=` so a hard refresh / shareable URL still works
    // for text-only prompts; attachments are registry-only because a
    // multi-megabyte dataUrl can't ride a URL search param. The chat
    // screen prefers the registry when both are present.
    setPendingInitialMessage({
      agentId: selectedAgentId,
      text: input.text,
      attachments: input.attachments,
      createdAt: Date.now(),
    })
    navigate(
      `/home/agents/${selectedAgentId}?q=${encodeURIComponent(input.text)}`,
    )
  }

  const handleSelectAgent = (agent: AgentEntry) => {
    setSelectedAgentId(agent.agentId)
  }

  const selectedAgent = legacyAgents.find(
    (agent) => agent.agentId === selectedAgentId,
  )
  const selectedAgentReady = selectedAgent
    ? selectedAgent.source === 'agent-harness' || status?.status === 'running'
    : false
  const selectedAgentStatus =
    selectedAgent?.source === 'agent-harness' ? 'running' : status?.status
  const selectedAgentName =
    selectedAgent?.name ?? orderedAgents[0]?.name ?? 'your agent'

  const hasAgents = legacyAgents.length > 0

  return (
    <div className="min-h-full px-4 py-6">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-8">
        {hasAgents ? (
          <>
            <div className="flex flex-col items-center gap-5 pt-[max(10vh,24px)] text-center">
              <div className="space-y-3">
                <h1 className="font-semibold text-[clamp(2.25rem,4.5vw,3.5rem)] leading-[1.08] tracking-[-0.025em] [text-wrap:balance]">
                  What should your agent{' '}
                  <span className="font-medium text-[var(--accent-orange)] italic">
                    work on
                  </span>{' '}
                  next?
                </h1>
                <p className="mx-auto max-w-2xl text-muted-foreground text-sm leading-6 [text-wrap:pretty]">
                  Start a task, continue a thread, or hand off to a different
                  agent — all without leaving this tab.
                </p>
              </div>

              <div className="w-full max-w-3xl">
                <ConversationInput
                  variant="home"
                  agents={legacyAgents}
                  selectedAgentId={selectedAgentId}
                  onSelectAgent={handleSelectAgent}
                  onSend={handleSend}
                  onCreateAgent={() => navigate('/agents')}
                  streaming={false}
                  disabled={!selectedAgentReady}
                  status={selectedAgentStatus}
                  attachmentsEnabled={true}
                  placeholder={
                    selectedAgentReady
                      ? `Ask ${selectedAgentName} to handle a task...`
                      : 'Agent runtime is not running...'
                  }
                />
              </div>
            </div>

            <Separator />

            <RecentThreads
              activeAgentId={selectedAgentId}
              agents={orderedAgents}
              adapters={adapters}
              onOpenAgents={() => navigate('/agents')}
              onSelectAgent={(agentId) => navigate(`/home/agents/${agentId}`)}
            />
          </>
        ) : (
          <EmptyAgentsState onOpenAgents={() => navigate('/agents')} />
        )}
      </div>

      {activeHint === 'signin' ? <SignInHint /> : null}
      {activeHint === 'import' ? <ImportDataHint /> : null}
    </div>
  )
}
