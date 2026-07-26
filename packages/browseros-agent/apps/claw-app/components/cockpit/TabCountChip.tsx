import type { SessionBrowserTab } from '@browseros/claw-api'
import { Layers } from 'lucide-react'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { siteOf } from '@/screens/cockpit/cockpit.helpers'

interface TabCountChipProps {
  browserTabs: SessionBrowserTab[]
  selectedBrowserTabId: number
}

/**
 * Lists every browser tab owned by one live session, highlighting the tab
 * currently surfaced on the card and showing its last tool when available.
 */
export function TabCountChip({
  browserTabs,
  selectedBrowserTabId,
}: TabCountChipProps) {
  if (browserTabs.length <= 1) return null
  return (
    <Popover>
      <PopoverTrigger
        data-tab-count={browserTabs.length}
        className="inline-flex shrink-0 items-center gap-1 rounded-full border border-border-2 bg-card px-1.5 py-[1.5px] font-bold text-[10px] text-ink-2 uppercase tracking-wider transition hover:border-border-strong"
      >
        <Layers className="size-2.5" />
        {browserTabs.length} tabs
      </PopoverTrigger>
      <PopoverContent className="w-72 p-2" align="end">
        <ul className="flex flex-col gap-1">
          {browserTabs.map((tab) => {
            const selected = tab.browserTabId === selectedBrowserTabId
            return (
              <li
                key={tab.browserTabId}
                className={
                  selected
                    ? 'flex items-start gap-2 rounded-md bg-card-tint px-2 py-1.5'
                    : 'flex items-start gap-2 rounded-md px-2 py-1.5 hover:bg-bg-sunken'
                }
              >
                <span
                  aria-hidden
                  className={
                    selected
                      ? 'mt-1 size-1.5 shrink-0 rounded-full bg-accent'
                      : 'mt-1 size-1.5 shrink-0 rounded-full bg-ink-4'
                  }
                />
                <div className="min-w-0 flex-1">
                  <div className="truncate font-semibold text-[12px]">
                    {tab.title || siteOf(tab.url)}
                  </div>
                  <div className="truncate font-mono text-[10.5px] text-ink-3">
                    {[tab.lastToolName, siteOf(tab.url)]
                      .filter(Boolean)
                      .join(' . ')}
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
