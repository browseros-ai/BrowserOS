import { Search } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Input } from '@/components/motion/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/motion/select'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import type { AgentOption } from '@/screens/audit/audit.helpers'
import type { HistoryView } from '@/screens/audit/audit.search-params'

const SEARCH_DEBOUNCE_MS = 250
const ALL_AGENTS = 'all'

interface FilterBarProps {
  search: string
  view: HistoryView
  agentSlug: string | null
  agentOptions: AgentOption[]
  onSearchChange: (search: string) => void
  onViewChange: (view: HistoryView) => void
  onAgentChange: (slug: string | null) => void
}

/** Search, All or Needs a look, and which agent: the only ways to narrow History. */
export function FilterBar({
  search,
  view,
  agentSlug,
  agentOptions,
  onSearchChange,
  onViewChange,
  onAgentChange,
}: FilterBarProps) {
  const [draft, setDraft] = useState(search)
  const [lastSearch, setLastSearch] = useState(search)
  const timer = useRef<number | undefined>(undefined)

  // "Show all tasks" clears the search from outside; follow it.
  if (search !== lastSearch) {
    setLastSearch(search)
    setDraft(search)
  }

  // A search still waiting to apply must not rewrite the next page's URL.
  useEffect(() => () => window.clearTimeout(timer.current), [])

  const typeSearch = (next: string) => {
    setDraft(next)
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(
      () => onSearchChange(next.trim()),
      SEARCH_DEBOUNCE_MS,
    )
  }

  return (
    <div className="flex flex-wrap items-center gap-2.5">
      <Input
        type="search"
        aria-label="Search tasks"
        placeholder="Search tasks"
        value={draft}
        onChange={typeSearch}
        leftIcon={<Search className="size-[18px]" />}
        className="min-w-[240px] flex-1"
        classNames={{
          field:
            'h-11 rounded-full border-border-strong bg-card focus-within:border-primary',
          input: 'text-[15px]',
        }}
      />
      <Tabs
        value={view}
        onValueChange={(next) =>
          onViewChange(next === 'attention' ? 'attention' : 'all')
        }
      >
        <TabsList aria-label="Which tasks" className="border border-border-2">
          <TabsTrigger value="all" className="font-semibold text-[14px]">
            All
          </TabsTrigger>
          <TabsTrigger value="attention" className="font-semibold text-[14px]">
            Needs a look
          </TabsTrigger>
        </TabsList>
      </Tabs>
      {agentOptions.length > 1 || agentSlug ? (
        <Select
          value={agentSlug ?? ALL_AGENTS}
          onValueChange={(next) =>
            onAgentChange(next === ALL_AGENTS ? null : next)
          }
          className="w-48"
        >
          <SelectTrigger className="h-10 font-semibold text-[14px] text-ink-2">
            <SelectValue placeholder="All agents" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL_AGENTS}>All agents</SelectItem>
            {agentOptions.map((agent) => (
              <SelectItem key={agent.slug} value={agent.slug}>
                {agent.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : null}
    </div>
  )
}
