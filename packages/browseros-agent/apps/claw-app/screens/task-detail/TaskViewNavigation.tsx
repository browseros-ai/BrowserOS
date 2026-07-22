import { type ReactNode, useState } from 'react'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

export interface TaskViewNavigationItem {
  id: string
  label: string
  count: number
  content: ReactNode
}

interface TaskViewLabelProps {
  item: TaskViewNavigationItem
}

function TaskViewLabel({ item }: TaskViewLabelProps) {
  return (
    <span className="flex min-w-0 flex-1 items-center justify-between gap-3">
      <span className="truncate">{item.label}</span>
      <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 font-mono text-[10.5px] text-ink-3">
        {item.count}
        <span className="sr-only"> events</span>
      </span>
    </span>
  )
}

/** Compact scope navigation for aggregate and per-browser-tab Audit views. */
export function TaskViewNavigation({
  items,
  defaultId,
}: {
  items: TaskViewNavigationItem[]
  defaultId?: string
}) {
  const [selectedId, setSelectedId] = useState(defaultId ?? items[0]?.id)
  const first = items[0]

  if (!first) return null
  if (items.length === 1) return <div>{first.content}</div>

  const selected =
    items.find((item) => item.id === selectedId) ??
    items.find((item) => item.id === defaultId) ??
    first

  return (
    <div className="space-y-4" data-task-view-navigation={items.length}>
      <div className="flex min-w-0 items-center gap-3">
        <span className="shrink-0 font-semibold text-ink-3 text-xs">View</span>
        <Select
          value={selected.id}
          onValueChange={(value) => {
            if (value !== null) setSelectedId(value)
          }}
        >
          <SelectTrigger
            aria-label="Audit view"
            className="w-full max-w-72 bg-card"
          >
            <SelectValue>
              <TaskViewLabel item={selected} />
            </SelectValue>
          </SelectTrigger>
          <SelectContent
            align="start"
            alignItemWithTrigger={false}
            className="max-h-[min(18rem,var(--available-height))] min-w-56 overflow-y-auto"
          >
            {items.map((item) => (
              <SelectItem key={item.id} value={item.id} label={item.label}>
                <TaskViewLabel item={item} />
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {selected.content}
    </div>
  )
}
