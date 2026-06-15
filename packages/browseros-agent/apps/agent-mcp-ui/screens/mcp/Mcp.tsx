import { PlugZap, Plus } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import type { AgentProfile } from '@/modules/api/agents.hooks'
import { McpRow } from './McpRow'
import { useMcpRegistryData } from './mcp.data'

export function Mcp() {
  const { profiles, isLoading, regenerate, navigate } = useMcpRegistryData()

  const onAdd = () => navigate('/agents/new')
  const onRegenerate = (profile: AgentProfile) =>
    regenerate.mutate({ id: profile.id })

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col px-8 pt-6 pb-20">
      <header className="mb-5 flex items-start gap-3">
        <span className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-accent-tint text-accent">
          <PlugZap className="size-5" />
        </span>
        <div className="flex-1">
          <div className="flex items-center gap-2">
            <h1 className="font-extrabold text-2xl text-ink tracking-tight">
              MCP
            </h1>
            {profiles.length > 0 && (
              <span className="inline-flex items-center gap-1.5 rounded-full bg-accent-tint px-2.5 py-0.5 font-bold text-accent-ink text-xs">
                {profiles.length} endpoint{profiles.length === 1 ? '' : 's'}
              </span>
            )}
          </div>
          <p className="mt-0.5 text-ink-2 text-sm">
            Every agent profile gets a slug-routed MCP endpoint. Copy the URL
            into your harness, or use one-click add if the harness exposes a
            connector install API.
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
        <div className="flex flex-col gap-2.5">
          {profiles.map((profile) => (
            <McpRow
              key={profile.id}
              profile={profile}
              isRegenerating={
                regenerate.isPending && regenerate.variables?.id === profile.id
              }
              onRegenerate={onRegenerate}
            />
          ))}
        </div>
      )}
    </div>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components, kept private to the registry screen.
 * -------------------------------------------------------------------------*/

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="flex flex-col items-start gap-3 rounded-2xl border border-border border-dashed bg-card px-6 py-10">
      <span className="flex size-9 items-center justify-center rounded-lg bg-accent-tint text-accent-ink">
        <PlugZap className="size-4" />
      </span>
      <h2 className="font-bold text-ink text-lg tracking-tight">
        No endpoints yet
      </h2>
      <p className="max-w-md text-ink-3 text-sm leading-snug">
        Endpoints land here when you add an agent profile. Each profile gets a
        unique slug-routed MCP URL plus a one-click "Add to harness" handoff.
      </p>
      <Button type="button" onClick={onAdd}>
        <Plus className="size-4" />
        Add your first agent
      </Button>
    </div>
  )
}
