import { Bot, Plus } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import type { AgentProfile } from '@/modules/api/agents.hooks'
import { AgentDirectoryRow } from './AgentDirectoryRow'
import { useAgentsDirectoryData } from './agents.data'
import { DeleteAgentDialog } from './DeleteAgentDialog'

export function Agents() {
  const { profiles, isLoading, deleteAgent, navigate } =
    useAgentsDirectoryData()
  const [pendingDelete, setPendingDelete] = useState<AgentProfile | null>(null)

  const onAdd = () => navigate('/agents/new')
  const onEdit = (profile: AgentProfile) =>
    navigate(`/agents/${profile.id}/edit`)
  const onRevoke = (profile: AgentProfile) => setPendingDelete(profile)
  const onConfirmRevoke = (profile: AgentProfile) => {
    deleteAgent.mutate(
      { id: profile.id },
      {
        onSettled: () => setPendingDelete(null),
      },
    )
  }

  const liveCount = profiles.filter(
    (profile) => profile.status === 'configured',
  ).length

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col px-8 pt-6 pb-20">
      <header className="mb-5 flex items-start gap-3">
        <span className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-accent-tint text-accent">
          <Bot className="size-5" />
        </span>
        <div className="flex-1">
          <div className="flex items-center gap-2">
            <h1 className="font-extrabold text-2xl text-ink tracking-tight">
              Agents
            </h1>
            {profiles.length > 0 && (
              <span className="inline-flex items-center gap-1.5 rounded-full bg-green-tint px-2.5 py-0.5 font-bold text-green text-xs">
                {liveCount} configured
              </span>
            )}
          </div>
          <p className="mt-0.5 text-ink-2 text-sm">
            Every connector you've registered with BrowserClaw, with its login
            scope, guardrails, and MCP endpoint.
          </p>
        </div>
        <Button type="button" onClick={onAdd}>
          <Plus className="size-4" />
          Add agent
        </Button>
      </header>

      {isLoading ? (
        <div className="flex justify-center py-12 text-ink-3">
          <Spinner />
        </div>
      ) : profiles.length === 0 ? (
        <EmptyState onAdd={onAdd} />
      ) : (
        <div className="flex flex-col gap-2">
          {profiles.map((profile) => (
            <AgentDirectoryRow
              key={profile.id}
              profile={profile}
              onEdit={onEdit}
              onRevoke={onRevoke}
            />
          ))}
        </div>
      )}

      <DeleteAgentDialog
        profile={pendingDelete}
        isDeleting={deleteAgent.isPending}
        onConfirm={onConfirmRevoke}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components, kept private to the directory screen.
 * -------------------------------------------------------------------------*/

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="flex flex-col items-start gap-3 rounded-2xl border border-border border-dashed bg-card px-6 py-10">
      <span className="flex size-9 items-center justify-center rounded-lg bg-accent-tint text-accent-ink">
        <Bot className="size-4" />
      </span>
      <h2 className="font-bold text-ink text-lg tracking-tight">
        No agents yet
      </h2>
      <p className="max-w-md text-ink-3 text-sm leading-snug">
        Connect a harness like Claude Code, Codex, or Hermes to BrowserClaw.
        Each agent gets its own login scope, approval rules, and MCP endpoint.
      </p>
      <Button type="button" onClick={onAdd}>
        <Plus className="size-4" />
        Add your first agent
      </Button>
    </div>
  )
}
