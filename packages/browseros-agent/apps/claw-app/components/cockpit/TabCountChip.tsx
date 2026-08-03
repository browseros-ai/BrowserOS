import type { SessionBrowserTab } from '@browseros/claw-api'
import { Layers } from 'lucide-react'
import { useState } from 'react'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { siteOf } from '@/screens/cockpit/cockpit.helpers'

interface TabCountChipProps {
  browserTabs: SessionBrowserTab[]
  /** The tab currently surfaced on the card. */
  selectedBrowserTabId: number
  /** The session's live target tab, marked "on screen" in the list. */
  liveBrowserTabId?: number
  onSelectTab: (browserTabId: number) => void
}

/**
 * Switches which owned tab the card previews. Each row selects that tab; the
 * shown tab is highlighted and the session's live target is marked "on screen".
 */
export function TabCountChip({
  browserTabs,
  selectedBrowserTabId,
  liveBrowserTabId,
  onSelectTab,
}: TabCountChipProps) {
  const [open, setOpen] = useState(false)
  if (browserTabs.length <= 1) return null
  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger
        data-tab-count={browserTabs.length}
        className="inline-flex shrink-0 items-center gap-1 rounded-full border border-border-2 bg-card px-1.5 py-[1.5px] font-bold text-[10px] text-ink-2 uppercase tracking-wider transition hover:border-border-strong"
      >
        <Layers className="size-2.5" />
        {browserTabs.length} tabs
      </PopoverTrigger>
      <PopoverContent className="ph-no-capture w-72 p-1.5" align="end">
        <ul className="flex flex-col gap-0.5">
          {browserTabs.map((tab) => {
            const selected = tab.browserTabId === selectedBrowserTabId
            const isLive = tab.browserTabId === liveBrowserTabId
            return (
              <li key={tab.browserTabId}>
                <button
                  type="button"
                  aria-current={selected}
                  data-select-browser-tab={tab.browserTabId}
                  onClick={() => {
                    onSelectTab(tab.browserTabId)
                    setOpen(false)
                  }}
                  className={
                    selected
                      ? 'flex w-full items-start gap-2 rounded-md bg-card-tint px-2 py-1.5 text-left'
                      : 'flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left transition hover:bg-bg-sunken'
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
                        .join(' · ')}
                    </div>
                  </div>
                  {isLive && (
                    <span className="mt-0.5 shrink-0 font-mono text-[9px] text-accent uppercase tracking-[0.08em]">
                      on screen
                    </span>
                  )}
                </button>
              </li>
            )
          })}
        </ul>
      </PopoverContent>
    </Popover>
  )
}
