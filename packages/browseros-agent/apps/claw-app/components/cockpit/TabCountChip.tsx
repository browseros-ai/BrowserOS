import type { SessionBrowserTab } from '@browseros/claw-api'
import { ArrowUpRight, ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { Favicon } from '@/components/favicon/Favicon'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { cn } from '@/lib/utils'
import { siteOf } from '@/screens/cockpit/cockpit.helpers'

interface TabCountChipProps {
  browserTabs: SessionBrowserTab[]
  /** The tab the card's preview shows, marked with a dot in the list. */
  onScreenBrowserTabId?: number
  /** Brings that browser tab to the front of its window. */
  onGoToTab: (browserTabId: number) => void
}

/**
 * How many tabs an agent is working in, as quiet text that opens the list of
 * them. Picking a row takes the person to that browser tab.
 */
export function TabCountChip({
  browserTabs,
  onScreenBrowserTabId,
  onGoToTab,
}: TabCountChipProps) {
  const [open, setOpen] = useState(false)
  if (browserTabs.length <= 1) return null
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        data-tab-count={browserTabs.length}
        className="inline-flex shrink-0 cursor-pointer items-center gap-[3px] rounded-sm font-semibold text-ink leading-[18px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        {browserTabs.length} tabs
        <ChevronDown
          aria-hidden
          strokeWidth={2.25}
          className={cn(
            'size-3 text-ink-2 transition-transform',
            open && 'rotate-180',
          )}
        />
      </PopoverTrigger>
      {/* The list portals out of the card's ph-no-capture section, so it carries its own. */}
      <PopoverContent
        align="start"
        alignOffset={-2}
        sideOffset={8}
        className="ph-no-capture max-h-(--available-height) w-[270px] gap-px overflow-y-auto rounded-lg border border-border-2 bg-card p-[5px] shadow-pop ring-0"
      >
        <ul className="flex flex-col gap-px">
          {browserTabs.map((tab) => {
            const title = tab.title || siteOf(tab.url)
            const onScreen = tab.browserTabId === onScreenBrowserTabId
            return (
              <li key={tab.browserTabId}>
                <button
                  type="button"
                  aria-label={`Go to ${title}`}
                  aria-current={onScreen || undefined}
                  data-go-to-browser-tab={tab.browserTabId}
                  onClick={() => {
                    onGoToTab(tab.browserTabId)
                    setOpen(false)
                  }}
                  className="group/row flex w-full cursor-pointer items-center gap-[9px] rounded-[6px] px-2 py-[7px] text-left outline-none hover:bg-card-tint focus-visible:bg-card-tint"
                >
                  <Favicon site={tab.url} className="size-4 rounded-[4px]" />
                  <span className="min-w-0 flex-1 truncate text-[13.5px] text-cyanotype-ink leading-[17px]">
                    {title}
                  </span>
                  <span
                    aria-hidden
                    className="flex w-3.5 shrink-0 justify-center"
                  >
                    {onScreen ? (
                      <span className="size-1.5 rounded-full bg-primary group-hover/row:hidden group-focus-visible/row:hidden" />
                    ) : null}
                    <ArrowUpRight
                      strokeWidth={2.25}
                      className="hidden size-3 text-ink-2 group-hover/row:block group-focus-visible/row:block"
                    />
                  </span>
                </button>
              </li>
            )
          })}
        </ul>
      </PopoverContent>
    </Popover>
  )
}
