import { Bot, Check, ChevronDown, Plus } from 'lucide-react'
import type { FC } from 'react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import {
  type AgentEntry,
  getModelDisplayName,
} from '@/entrypoints/app/agents/useOpenClaw'
import { cn } from '@/lib/utils'

interface AgentSelectorProps {
  agents: AgentEntry[]
  selectedAgentId: string | null
  onSelectAgent: (agent: AgentEntry) => void
  onCreateAgent?: () => void
  status?: string
  /**
   * `'pill'` renders the filled-pill variant used by the calm
   * composer on `/home` — bordered, slightly elevated background,
   * mono agent name, used as the visual anchor on the left of the
   * footer chip row. Default `'ghost'` keeps the existing flat
   * shadcn ghost-button trigger used by the chat surface.
   */
  triggerVariant?: 'ghost' | 'pill'
}

function getStatusDot(status?: string) {
  if (status === 'running') return 'bg-emerald-500'
  if (status === 'starting') return 'bg-amber-500 animate-pulse'
  if (status === 'error') return 'bg-destructive'
  return 'bg-muted-foreground/50'
}

export const AgentSelector: FC<AgentSelectorProps> = ({
  agents,
  selectedAgentId,
  onSelectAgent,
  onCreateAgent,
  status,
  triggerVariant = 'ghost',
}) => {
  const [open, setOpen] = useState(false)
  const selectedAgent = agents.find(
    (agent) => agent.agentId === selectedAgentId,
  )

  const triggerNode =
    triggerVariant === 'pill' ? (
      <button
        type="button"
        className={cn(
          'inline-flex h-6 max-w-[180px] items-center gap-1.5 rounded-full border border-border bg-accent/40 pr-2 pl-2.5 text-[11.5px] text-foreground transition-colors',
          'hover:border-border hover:bg-accent/70 data-[state=open]:border-border data-[state=open]:bg-accent/70',
        )}
      >
        <span className={cn('size-1.5 rounded-full', getStatusDot(status))} />
        <span className="truncate font-medium font-mono text-[11.5px] tracking-[-0.01em]">
          {selectedAgent?.name ?? 'Select agent'}
        </span>
        <ChevronDown className="size-3 shrink-0 text-muted-foreground" />
      </button>
    ) : (
      <Button
        variant="ghost"
        className={cn(
          'flex items-center gap-2 rounded-lg px-3 py-1.5 font-medium text-sm transition-all',
          'bg-transparent text-muted-foreground hover:bg-accent hover:text-accent-foreground',
          'data-[state=open]:bg-accent',
        )}
      >
        <Bot className="h-4 w-4" />
        <span className={cn('size-2 rounded-full', getStatusDot(status))} />
        <span className="max-w-32 truncate">
          {selectedAgent?.name ?? 'Select agent'}
        </span>
        <ChevronDown className="h-3 w-3" />
      </Button>
    )

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{triggerNode}</PopoverTrigger>
      <PopoverContent side="bottom" align="start" className="w-72 p-0">
        <Command>
          <CommandInput placeholder="Search agents..." className="h-9" />
          <CommandList>
            <CommandEmpty>No agents found</CommandEmpty>
            <CommandGroup>
              {agents.map((agent) => {
                const isSelected = selectedAgentId === agent.agentId
                const modelLabel = getModelDisplayName(agent.model)
                return (
                  <CommandItem
                    key={agent.agentId}
                    value={`${agent.agentId} ${agent.name}`}
                    onSelect={() => {
                      onSelectAgent(agent)
                      setOpen(false)
                    }}
                    className={cn(
                      'flex w-full items-center gap-3 rounded-md px-3 py-2',
                      isSelected && 'bg-[var(--accent-orange)]/10',
                    )}
                  >
                    <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]">
                      <Bot className="size-4" />
                    </div>
                    <div className="min-w-0 flex-1">
                      <span className="block truncate font-medium text-sm">
                        {agent.name}
                      </span>
                      {modelLabel ? (
                        <span className="block truncate text-muted-foreground text-xs">
                          {modelLabel}
                        </span>
                      ) : null}
                    </div>
                    {isSelected ? (
                      <Check className="size-4 shrink-0 text-[var(--accent-orange)]" />
                    ) : null}
                  </CommandItem>
                )
              })}
            </CommandGroup>
            {onCreateAgent ? (
              <div className="border-border border-t p-1">
                <button
                  type="button"
                  className="flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-muted-foreground text-sm transition-colors hover:bg-accent hover:text-foreground"
                  onClick={() => {
                    onCreateAgent()
                    setOpen(false)
                  }}
                >
                  <Plus className="size-4" />
                  <span>Create agent</span>
                </button>
              </div>
            ) : null}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
