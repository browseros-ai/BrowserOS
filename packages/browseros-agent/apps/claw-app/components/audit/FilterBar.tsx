import { Check, ChevronDown, Search, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import type { TaskStatus } from '@/modules/api/audit.hooks'
import type { AgentChip } from '@/screens/audit/audit.helpers'
import { AgentDot } from './AgentDot'
import { StatusBadge } from './StatusBadge'

const SEARCH_DEBOUNCE_MS = 250

interface FilterBarProps {
  agentOptions: AgentChip[]
  statusOptions: { status: TaskStatus; count: number }[]
  siteOptions: { site: string; count: number }[]
  selectedAgentId: string | null
  selectedStatus: TaskStatus | null
  selectedSite: string | null
  search: string
  onAgentChange: (agentId: string | null) => void
  onStatusChange: (status: TaskStatus | null) => void
  onSiteChange: (site: string | null) => void
  onSearchChange: (q: string) => void
}

export function FilterBar({
  agentOptions,
  statusOptions,
  siteOptions,
  selectedAgentId,
  selectedStatus,
  selectedSite,
  search,
  onAgentChange,
  onStatusChange,
  onSiteChange,
  onSearchChange,
}: FilterBarProps) {
  const selectedAgent = agentOptions.find((a) => a.agentId === selectedAgentId)
  // Local search state so each keystroke updates the input
  // immediately, but the URL + refetch only fires after the operator
  // has paused typing. Without this every character triggered a
  // re-render + network request, stacking up while the user typed.
  const [localSearch, setLocalSearch] = useState(search)
  useEffect(() => {
    setLocalSearch(search)
  }, [search])
  useEffect(() => {
    if (localSearch === search) return
    const id = setTimeout(() => onSearchChange(localSearch), SEARCH_DEBOUNCE_MS)
    return () => clearTimeout(id)
  }, [localSearch, search, onSearchChange])

  const clearSearch = (): void => {
    setLocalSearch('')
    onSearchChange('')
  }

  return (
    <div className="flex flex-wrap items-center gap-1 border-border-2 border-y py-2.5">
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              size="sm"
              className="h-8 gap-1.5 font-mono text-[11px] text-ink-2 uppercase tracking-[0.08em] hover:bg-card-tint"
            />
          }
        >
          {selectedAgent ? (
            <>
              <AgentDot slug={selectedAgent.slug} />
              {selectedAgent.agentLabel}
            </>
          ) : (
            'Agent'
          )}
          <ChevronDown className="size-3 text-ink-3" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-52">
          <DropdownMenuItem onClick={() => onAgentChange(null)}>
            <span className="flex-1">All</span>
            {selectedAgentId === null && <Check className="size-3.5" />}
          </DropdownMenuItem>
          {agentOptions.map((opt) => (
            <DropdownMenuItem
              key={opt.agentId}
              onClick={() => onAgentChange(opt.agentId)}
            >
              <AgentDot slug={opt.slug} className="mr-1.5" />
              <span className="flex-1">{opt.agentLabel}</span>
              <span className="ml-2 text-[11.5px] text-ink-3">{opt.count}</span>
              {selectedAgentId === opt.agentId && (
                <Check className="ml-2 size-3.5" />
              )}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              size="sm"
              className="h-8 gap-1.5 font-mono text-[11px] text-ink-2 uppercase tracking-[0.08em] hover:bg-card-tint"
            />
          }
        >
          {selectedStatus ? <StatusPill status={selectedStatus} /> : 'Status'}
          <ChevronDown className="size-3 text-ink-3" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-44">
          <DropdownMenuItem onClick={() => onStatusChange(null)}>
            <span className="flex-1">All</span>
            {selectedStatus === null && <Check className="size-3.5" />}
          </DropdownMenuItem>
          {statusOptions.map((opt) => (
            <DropdownMenuItem
              key={opt.status}
              onClick={() => onStatusChange(opt.status)}
            >
              <StatusBadge status={opt.status} className="mr-2" />
              <span className="ml-1 text-[11.5px] text-ink-3">{opt.count}</span>
              {selectedStatus === opt.status && (
                <Check className="ml-auto size-3.5" />
              )}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      {siteOptions.length > 0 && (
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant="ghost"
                size="sm"
                className="h-8 gap-1.5 font-mono text-[11px] text-ink-2 uppercase tracking-[0.08em] hover:bg-card-tint"
              />
            }
          >
            {selectedSite ?? 'Site'}
            <ChevronDown className="size-3 text-ink-3" />
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="start"
            className="max-h-64 min-w-52 overflow-y-auto"
          >
            <DropdownMenuItem onClick={() => onSiteChange(null)}>
              <span className="flex-1">All</span>
              {selectedSite === null && <Check className="size-3.5" />}
            </DropdownMenuItem>
            {siteOptions.map((opt) => (
              <DropdownMenuItem
                key={opt.site}
                onClick={() => onSiteChange(opt.site)}
              >
                <span className="flex-1 truncate">{opt.site}</span>
                <span className="ml-2 text-[11.5px] text-ink-3">
                  {opt.count}
                </span>
                {selectedSite === opt.site && (
                  <Check className="ml-2 size-3.5" />
                )}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      )}

      <div className="relative ml-auto flex items-center">
        <Search className="absolute left-2.5 size-3.5 text-ink-3" aria-hidden />
        <Input
          value={localSearch}
          onChange={(e) => setLocalSearch(e.target.value)}
          placeholder="search sessions..."
          // pr-7 reserves space for the inline clear button so the
          // text never sits under the icon.
          className="h-8 w-64 border-none bg-transparent pr-7 pl-8 font-mono text-[11.5px] text-ink-1 placeholder:text-ink-3 focus-visible:bg-card-tint focus-visible:ring-0"
        />
        {localSearch.length > 0 && (
          <button
            type="button"
            onClick={clearSearch}
            aria-label="Clear search"
            data-testid="filter-search-clear"
            className="absolute right-1.5 inline-flex size-5 items-center justify-center rounded text-ink-3 transition-colors hover:bg-card-tint hover:text-ink-1"
          >
            <X className="size-3.5" />
          </button>
        )}
      </div>
    </div>
  )
}

function StatusPill({ status }: { status: TaskStatus }) {
  if (status === 'live') {
    return (
      <span className="inline-flex items-center gap-1 text-accent">
        <span
          aria-hidden
          className="inline-block size-1.5 animate-[pulse-dot_1.4s_ease-in-out_infinite] rounded-full bg-accent"
        />
        Live
      </span>
    )
  }
  if (status === 'failed') {
    return (
      <span className="inline-flex items-center gap-1 text-red-500">
        <span
          aria-hidden
          className="inline-block size-1.5 rounded-full bg-red-500"
        />
        Failed
      </span>
    )
  }
  return <span>Done</span>
}
