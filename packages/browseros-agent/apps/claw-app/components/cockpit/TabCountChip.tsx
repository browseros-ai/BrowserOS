import { Layers } from 'lucide-react'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import type { TabActivityRecord } from '@/modules/api/tabs.hooks'
import { siteOf } from '@/screens/cockpit/cockpit.helpers'

interface TabCountChipProps {
  tabs: TabActivityRecord[]
  /** Target id of the focus tab so the popover can highlight it. */
  focusTargetId: string
}

/**
 * "N tabs" chip on the AgentRunningCard header. Click to open a
 * popover listing every tab this agent owns, with the current focus
 * highlighted and the last tool name shown per row.
 */
export function TabCountChip({ tabs, focusTargetId }: TabCountChipProps) {
  if (tabs.length <= 1) return null
  return (
    <Popover>
      <PopoverTrigger
        data-tab-count={tabs.length}
        className="inline-flex shrink-0 items-center gap-1 rounded-full border border-border-2 bg-card px-1.5 py-[1.5px] font-bold text-[10px] text-ink-2 uppercase tracking-wider transition hover:border-border-strong"
      >
        <Layers className="size-2.5" />
        {tabs.length} tabs
      </PopoverTrigger>
      <PopoverContent className="w-72 p-2" align="end">
        <ul className="flex flex-col gap-1">
          {tabs.map((tab) => {
            const isFocus = tab.targetId === focusTargetId
            return (
              <li
                key={tab.targetId}
                className={
                  isFocus
                    ? 'flex items-start gap-2 rounded-md bg-card-tint px-2 py-1.5'
                    : 'flex items-start gap-2 rounded-md px-2 py-1.5 hover:bg-bg-sunken'
                }
              >
                <span
                  aria-hidden
                  className={
                    isFocus
                      ? 'mt-1 size-1.5 shrink-0 rounded-full bg-accent'
                      : 'mt-1 size-1.5 shrink-0 rounded-full bg-ink-4'
                  }
                />
                <div className="min-w-0 flex-1">
                  <div className="truncate font-semibold text-[12px]">
                    {tab.title || siteOf(tab.url)}
                  </div>
                  <div className="truncate font-mono text-[10.5px] text-ink-3">
                    {tab.lastToolName} . {siteOf(tab.url)}
                  </div>
                </div>
              </li>
            )
          })}
        </ul>
      </PopoverContent>
    </Popover>
  )
}
